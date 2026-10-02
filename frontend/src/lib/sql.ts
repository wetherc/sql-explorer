import { format as layOutStatement, type SqlLanguage } from 'sql-formatter'
import { Dialect } from '@/types/api'

/**
 * The words the editor offers and highlights. The list holds the words
 * that the four engines share.
 */
export const SQL_KEYWORDS: readonly string[] = [
  'ADD',
  'ALL',
  'ALTER',
  'AND',
  'AS',
  'ASC',
  'BEGIN',
  'BETWEEN',
  'BY',
  'CASE',
  'CAST',
  'COMMIT',
  'CREATE',
  'CROSS',
  'DELETE',
  'DESC',
  'DISTINCT',
  'DROP',
  'ELSE',
  'END',
  'EXCEPT',
  'EXISTS',
  'FROM',
  'FULL',
  'GROUP',
  'HAVING',
  'IN',
  'INNER',
  'INSERT',
  'INTERSECT',
  'INTO',
  'IS',
  'JOIN',
  'LEFT',
  'LIKE',
  'LIMIT',
  'NOT',
  'NULL',
  'OFFSET',
  'ON',
  'OR',
  'ORDER',
  'OUTER',
  'OUTPUT',
  'RIGHT',
  'ROLLBACK',
  'SELECT',
  'SET',
  'TABLE',
  'THEN',
  'TOP',
  'UNION',
  'UPDATE',
  'USING',
  'VALUES',
  'VIEW',
  'WHEN',
  'WHERE',
  'WITH',
]

/** Wraps a name in the quotes the engine uses. */
export function quoteIdentifier(name: string, dialect: Dialect): string {
  switch (dialect) {
    case Dialect.MsSql:
      return `[${name.replace(/]/g, ']]')}]`
    case Dialect.MySql:
      return `\`${name.replace(/`/g, '``')}\``
    default:
      return `"${name.replace(/"/g, '""')}"`
  }
}

/**
 * True when the name needs no quotes, which holds when it starts with a
 * letter or an underscore and holds only letters, digits and underscores.
 */
export function isPlainIdentifier(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(name)
}

/**
 * The reserved words that most engines refuse as a name without quotes. The
 * list is short and covers the words that a table or a column often has,
 * such as `order` or `group`. A word that an engine accepts gets quotes all
 * the same, and a quoted name works on every engine.
 */
const SHARED_RESERVED = (
  'ALL ALTER AND ANY AS ASC BETWEEN BY CASE CAST CHECK COLUMN CONSTRAINT ' +
  'CREATE CROSS CURRENT_DATE CURRENT_TIME CURRENT_TIMESTAMP CURRENT_USER ' +
  'DEFAULT DELETE DESC DISTINCT DROP ELSE END EXCEPT EXISTS FALSE FOR ' +
  'FOREIGN FROM FULL GRANT GROUP HAVING IN INNER INSERT INTERSECT INTO IS ' +
  'JOIN LEFT LIKE NOT NULL ON OR ORDER OUTER PRIMARY REFERENCES RIGHT SELECT ' +
  'SET TABLE THEN TO TRUE UNION UNIQUE UPDATE USING VALUES WHEN WHERE WITH'
).split(' ')

/** The reserved words of one engine that the shared list does not have. */
const DIALECT_RESERVED: Record<Dialect, string> = {
  [Dialect.MsSql]:
    'BACKUP BEGIN CLOSE COMMIT DATABASE DECLARE EXEC EXECUTE FETCH FILE ' +
    'FUNCTION IDENTITY INDEX KEY OPEN PERCENT PLAN PROC PROCEDURE PUBLIC ' +
    'ROLLBACK RULE SCHEMA TOP TRAN TRANSACTION USER VIEW',
  [Dialect.MySql]:
    'CHANGE DATABASE DESCRIBE DIV GROUPS INDEX INTERVAL KEY KEYS LIMIT LOCK ' +
    'MOD OVER PROCEDURE RANGE RANK READ REPLACE ROW_NUMBER SCHEMA SHOW ' +
    'TRIGGER WINDOW WRITE',
  [Dialect.Postgres]:
    'ANALYSE ANALYZE ARRAY BOTH COLLATE CURRENT_ROLE DO FETCH LATERAL ' +
    'LEADING LIMIT LOCALTIME LOCALTIMESTAMP OFFSET ONLY RETURNING ' +
    'SESSION_USER SOME TRAILING USER WINDOW',
  [Dialect.Sqlite]: 'AUTOINCREMENT ESCAPE GLOB INDEX ISNULL LIMIT NOTNULL OFFSET',
  [Dialect.Athena]:
    'CUBE DEALLOCATE DESCRIBE ESCAPE EXECUTE EXTRACT GROUPING LOCALTIME ' +
    'LOCALTIMESTAMP NATURAL PREPARE RECURSIVE ROLLUP TRIM UNNEST',
}

/** The reserved words of each engine, in capital letters. */
const RESERVED = {} as Record<Dialect, ReadonlySet<string>>
for (const [dialect, words] of Object.entries(DIALECT_RESERVED) as [Dialect, string][]) {
  RESERVED[dialect] = new Set([...SHARED_RESERVED, ...words.split(' ')])
}

/** True when the engine refuses the word as a name without quotes. */
export function isReservedWord(name: string, dialect: Dialect): boolean {
  return RESERVED[dialect].has(name.toUpperCase())
}

/**
 * Quotes a name only when it needs quotes. PostgreSQL folds a name without
 * quotes to lower case, so a name with a capital letter needs quotes there.
 * A reserved word of the engine, such as `order`, needs quotes too.
 */
export function quoteIfNeeded(name: string, dialect: Dialect): string {
  const folds = dialect === Dialect.Postgres && name !== name.toLowerCase()
  return isPlainIdentifier(name) && !folds && !isReservedWord(name, dialect)
    ? name
    : quoteIdentifier(name, dialect)
}

/**
 * Returns the statement that surrounds the given position. The editor uses
 * it to run the statement under the cursor when nothing is selected. A
 * position that no statement holds gives the statement in front of it, and a
 * script that holds no statement gives an empty text. On MS SQL Server the
 * GO batch is the unit, because the backend sends a batch whole, and the
 * word GO never travels with it. On MySQL a routine body under a `DELIMITER`
 * command keeps that command.
 *
 * The split follows the rules of the dialect: the quotes, the comments and
 * the terminator that the backend splitter knows. The backend splits again
 * before it sends anything to a server, so both splits find the same
 * bounds.
 */
export function statementAt(script: string, offset: number, dialect?: Dialect): string {
  const position = Math.max(0, Math.min(offset, script.length))
  const parts = statementSpans(script, dialect, true)
    .map(({ start, end, delimiter }) => ({
      start,
      end,
      text: withDelimiter(script.slice(start, end).trim(), delimiter),
    }))
    .filter((part) => part.text !== '')
  const first = parts[0]
  if (!first) {
    return ''
  }
  const covering = parts.find((part) => position >= part.start && position <= part.end)
  if (covering) {
    return covering.text
  }
  // The position stands in the empty space that follows a semicolon. The
  // statement in front of that space is the one the user means, so a cursor
  // after the last semicolon of a script runs the last statement alone.
  const before = parts.filter((part) => part.start <= position)
  return (before[before.length - 1] ?? first).text
}

/**
 * Puts a statement that a MySQL `DELIMITER` command bounds back inside that
 * command. The backend splits the text again on the terminator in force, so
 * a routine body that holds a semicolon reaches the server whole. A text
 * without a semicolon stays bare, and a plan request can put its keyword in
 * front of it.
 */
function withDelimiter(text: string, delimiter: string): string {
  if (delimiter === ';' || !text.includes(';')) {
    return text
  }
  return `DELIMITER ${delimiter}\n${text}${delimiter}`
}

/**
 * The statement that holds one place of a script, with that place counted
 * from the start of the statement.
 *
 * A reader of the text around the cursor works on this and not on the whole
 * script, so the cost of the work follows one statement and not the size of
 * the file.
 */
export function statementAround(
  script: string,
  offset: number,
  dialect?: Dialect,
): { text: string; offset: number } {
  const position = Math.max(0, Math.min(offset, script.length))
  // The bounds cover the whole script and the splitter gives at least one of
  // them, so one of them always holds the place. The last one stands in
  // until the walk finds it.
  const bounds = statementBounds(script, dialect)
  let held = bounds[bounds.length - 1] as [number, number]
  for (const bound of bounds) {
    if (position >= bound[0] && position <= bound[1]) {
      held = bound
      break
    }
  }
  const [start, end] = held
  return {
    text: script.slice(start, end),
    offset: Math.max(0, Math.min(position - start, end - start)),
  }
}

/**
 * The batch separator of MS SQL Server: the word GO alone on a line, with an
 * optional count of runs and an optional comment behind it.
 */
const BATCH_SEPARATOR = /^[ \t]*GO(?:[ \t]+\d+)?[ \t]*(?:--[^\n]*)?(?:\r?\n|$)/i

/**
 * Reads a batch separator that starts at the given position. Returns the
 * position after the line of the separator, or -1 when the line holds
 * something else.
 */
function batchSeparatorAt(script: string, index: number): number {
  const match = BATCH_SEPARATOR.exec(script.slice(index))
  return match ? index + match[0].length : -1
}

/** The word of the command of MySQL that changes the terminator. */
const DELIMITER_KEYWORD = 'DELIMITER'

/**
 * Reads a `DELIMITER` command that starts at the given position. The command
 * holds the word, blank space, and the new terminator, which runs to the next
 * blank space. Returns the new terminator and the position after the line, or
 * null when the line holds something else.
 */
function delimiterCommandAt(
  script: string,
  index: number,
): { delimiter: string; end: number } | null {
  const word = script.slice(index, index + DELIMITER_KEYWORD.length)
  if (word.toUpperCase() !== DELIMITER_KEYWORD) {
    return null
  }
  let cursor = index + DELIMITER_KEYWORD.length
  if (script.charAt(cursor) !== ' ' && script.charAt(cursor) !== '\t') {
    return null
  }
  while (script.charAt(cursor) === ' ' || script.charAt(cursor) === '\t') {
    cursor += 1
  }
  const start = cursor
  while (cursor < script.length && !/\s/.test(script.charAt(cursor))) {
    cursor += 1
  }
  const delimiter = script.slice(start, cursor)
  return delimiter === '' ? null : { delimiter, end: endOfLine(script, cursor) }
}

/** The rules of one dialect that the split of a script follows. */
interface SplitRules {
  /** A backslash starts an escape inside every string literal. */
  backslashEscapes: boolean
  /** A backslash starts an escape inside a string with the prefix `E`. */
  prefixedEscapes: boolean
  /** A number sign starts a comment that runs to the end of the line. */
  hashComments: boolean
  /** Brackets quote a name. */
  bracketQuotes: boolean
  /** A backtick quotes a name. */
  backtickQuotes: boolean
  /** A dollar sign starts a tagged string literal. */
  dollarQuotes: boolean
  /** A block comment can hold another block comment. */
  nestedBlockComments: boolean
  /** The word GO alone on a line ends a batch. */
  batchSeparator: boolean
  /** The word DELIMITER alone on a line changes the terminator. */
  delimiterCommand: boolean
  /** The `BEGIN ... END` body of a trigger holds semicolons. */
  triggerBodies: boolean
  /** The `BEGIN ATOMIC ... END` body of a routine holds semicolons. */
  atomicBodies: boolean
}

/**
 * The rules of the given dialect. A caller that names no dialect gets the
 * rules that every engine shares, with the backtick of MySQL, because a
 * script of an unknown engine can carry one.
 */
function splitRules(dialect?: Dialect): SplitRules {
  return {
    backslashEscapes: dialect === Dialect.MySql,
    prefixedEscapes: dialect === Dialect.Postgres,
    hashComments: dialect === Dialect.MySql,
    bracketQuotes: dialect === Dialect.MsSql,
    backtickQuotes: dialect === Dialect.MySql || dialect === undefined,
    dollarQuotes: dialect === Dialect.Postgres,
    nestedBlockComments: dialect === Dialect.MsSql || dialect === Dialect.Postgres,
    batchSeparator: dialect === Dialect.MsSql,
    delimiterCommand: dialect === Dialect.MySql,
    triggerBodies: dialect === Dialect.Sqlite,
    atomicBodies: dialect === Dialect.Postgres,
  }
}

/** A bare word that starts at the `lastIndex` of the pattern. */
const BARE_WORD = /[\p{L}\p{N}_$]+/uy

/**
 * Follows the words of one PostgreSQL statement to find a routine body in
 * the form `BEGIN ATOMIC ... END`. The body can hold `CASE ... END`, so each
 * `CASE` inside the body also waits for an `END`. The backend splitter
 * follows the same words.
 */
class BodyWords {
  /** The number of `BEGIN ATOMIC` and `CASE` words that have no `END` yet. */
  depth = 0
  private first = ''
  private previous = ''

  /** Reads the next bare word of the statement, in small letters. */
  read(word: string): void {
    if (this.first === '') {
      this.first = word
    }
    if (this.depth > 0 && word === 'case') {
      this.depth += 1
    } else if (this.depth > 0 && word === 'end') {
      this.depth -= 1
    } else if (word === 'atomic' && this.previous === 'begin' && this.first === 'create') {
      this.depth += 1
    }
    this.previous = word
  }
}

/**
 * True when two dashes at the given position start a comment that runs to
 * the end of the line. MySQL reads them so only when a blank, a control
 * character or the end of the text follows them, so `5--1` is a subtraction
 * there. Every other dialect reads two dashes as a comment at once. The rule
 * follows the scanner of the backend.
 */
export function opensDashComment(
  chars: ArrayLike<string>,
  index: number,
  dialect?: Dialect,
): boolean {
  if (chars[index] !== '-' || chars[index + 1] !== '-') {
    return false
  }
  const after = chars[index + 2]
  return dialect !== Dialect.MySql || after === undefined || /[\p{White_Space}\p{Cc}]/u.test(after)
}

/** True when the character can stand inside a bare name. */
function inAWord(character: string | undefined): boolean {
  return character !== undefined && /[\p{L}\p{N}_$]/u.test(character)
}

/**
 * True when a backslash starts an escape inside the quoted region that opens
 * at the given position. PostgreSQL reads so a string with the prefix `E`
 * alone, as in `E'it\'s'`.
 */
function escapesAt(script: string, index: number, rules: SplitRules): boolean {
  if (script[index] === '`') {
    return false
  }
  if (rules.backslashEscapes) {
    return true
  }
  return (
    rules.prefixedEscapes &&
    script[index] === "'" &&
    /[eE]/.test(script.charAt(index - 1)) &&
    !inAWord(script[index - 2])
  )
}

/** The position after the end of the line that holds the given position. */
function endOfLine(script: string, index: number): number {
  const stop = script.indexOf('\n', index)
  return stop === -1 ? script.length : stop + 1
}

/**
 * The position after a block comment that starts at the given position. A
 * comment that no end mark closes runs to the end of the script.
 */
function endOfBlockComment(script: string, index: number, nested: boolean): number {
  let cursor = index + 2
  let depth = 1
  while (cursor < script.length) {
    if (nested && script.startsWith('/*', cursor)) {
      depth += 1
      cursor += 2
      continue
    }
    if (script.startsWith('*/', cursor)) {
      depth -= 1
      cursor += 2
      if (depth === 0) {
        return cursor
      }
      continue
    }
    cursor += 1
  }
  return cursor
}

/**
 * The position after a region that the given quote opens. A doubled quote
 * stays inside the region.
 */
function endOfQuoted(
  script: string,
  index: number,
  quote: string,
  backslashEscapes: boolean,
): number {
  let cursor = index + 1
  while (cursor < script.length) {
    const character = script[cursor]
    if (backslashEscapes && character === '\\') {
      cursor += 2
      continue
    }
    if (character === quote) {
      if (script[cursor + 1] === quote) {
        cursor += 2
        continue
      }
      return cursor + 1
    }
    cursor += 1
  }
  return cursor
}

/**
 * The position after a name in brackets. A doubled closing bracket stays
 * inside the name.
 */
function endOfBracket(script: string, index: number): number {
  let cursor = index + 1
  while (cursor < script.length) {
    if (script[cursor] === ']') {
      if (script[cursor + 1] === ']') {
        cursor += 2
        continue
      }
      return cursor + 1
    }
    cursor += 1
  }
  return cursor
}

/**
 * The position after a string that a dollar tag encloses. Returns -1 when
 * the dollar sign opens no tag.
 */
function endOfDollarQuoted(script: string, index: number): number {
  // A tag starts as a name starts, so `$1$` holds the parameter `$1`.
  const match = /^\$(?:[\p{L}_][\p{L}\p{N}_]*)?\$/u.exec(script.slice(index))
  if (!match) {
    return -1
  }
  const tag = match[0]
  const stop = script.indexOf(tag, index + tag.length)
  return stop === -1 ? script.length : stop + tag.length
}

/**
 * Returns the start and the end of every statement in the script. On MS SQL
 * Server the word GO ends a batch and belongs to no statement, so it bounds
 * the statement in front of it and the text of it never reaches the server.
 * On MySQL the word DELIMITER changes the terminator, and its line belongs
 * to no statement either.
 */
export function statementBounds(script: string, dialect?: Dialect): Array<[number, number]> {
  return statementSpans(script, dialect).map(({ start, end }) => [start, end])
}

/** One statement of a script and the terminator that was in force for it. */
interface StatementSpan {
  start: number
  end: number
  delimiter: string
}

/**
 * True when the text starts a SQLite trigger whose body has no `END` yet.
 * A semicolon inside that body ends nothing, and the backend joins the same
 * fragments before it sends the trigger.
 */
function insideTriggerBody(text: string): boolean {
  return /^\s*CREATE\s+(TEMP\s+|TEMPORARY\s+)?TRIGGER\b/i.test(text) && !/\bEND\s*$/i.test(text)
}

/**
 * The walk behind `statementBounds`, which also keeps the terminator. With
 * `whole` set, a batch of MS SQL Server is one span, because the backend
 * sends a batch whole and a cut at a semicolon would run part of it. A
 * SQLite trigger is one span for the same reason.
 */
function statementSpans(script: string, dialect?: Dialect, whole = false): StatementSpan[] {
  const rules = splitRules(dialect)
  const bounds: StatementSpan[] = []
  let delimiter = ';'
  let start = 0
  let index = 0
  // True when the statement from `start` holds more than blank space and
  // comments. A comment above a DELIMITER line, as in a dump file, does not
  // hide the command.
  let codeSeen = false
  let words = new BodyWords()

  while (index < script.length) {
    const character = script[index]
    const next = script[index + 1]
    const atLineStart = index === 0 || script[index - 1] === '\n'

    // The DELIMITER command holds a whole line, and it can stand only where
    // a statement starts. A comment in front of it is dropped with it.
    if (rules.delimiterCommand && atLineStart && !codeSeen) {
      const command = delimiterCommandAt(script, index)
      if (command) {
        delimiter = command.delimiter
        start = command.end
        index = command.end
        continue
      }
    }
    if (rules.batchSeparator && atLineStart) {
      const after = batchSeparatorAt(script, index)
      if (after >= 0) {
        bounds.push({ start, end: index, delimiter })
        start = after
        index = after
        codeSeen = false
        continue
      }
    }
    if (opensDashComment(script, index, dialect)) {
      index = endOfLine(script, index)
      continue
    }
    if (rules.hashComments && character === '#') {
      index = endOfLine(script, index)
      continue
    }
    if (character === '/' && next === '*') {
      // MySQL runs the text of `/*!` and `/*M!` comments as code.
      codeSeen ||= rules.delimiterCommand && /^\/\*M?!/.test(script.slice(index, index + 4))
      index = endOfBlockComment(script, index, rules.nestedBlockComments)
      continue
    }
    if (script.charAt(index).trim() !== '') {
      codeSeen = true
    }
    if (character === "'" || character === '"' || (character === '`' && rules.backtickQuotes)) {
      index = endOfQuoted(script, index, character, escapesAt(script, index, rules))
      continue
    }
    if (character === '[' && rules.bracketQuotes) {
      index = endOfBracket(script, index)
      continue
    }
    // A dollar sign inside a name, as in `a$x$`, is part of the name.
    if (character === '$' && rules.dollarQuotes && !inAWord(script[index - 1])) {
      const after = endOfDollarQuoted(script, index)
      if (after >= 0) {
        index = after
        continue
      }
    }
    if (
      rules.atomicBodies &&
      /[\p{L}\p{N}_]/u.test(script.charAt(index)) &&
      !inAWord(script[index - 1])
    ) {
      BARE_WORD.lastIndex = index
      // The character itself matches the pattern, so the match is never null.
      const word = (BARE_WORD.exec(script) as RegExpExecArray)[0]
      words.read(word.toLowerCase())
      index += word.length
      continue
    }
    if (
      words.depth === 0 &&
      !(whole && rules.batchSeparator) &&
      script.startsWith(delimiter, index) &&
      !(whole && rules.triggerBodies && insideTriggerBody(script.slice(start, index)))
    ) {
      bounds.push({ start, end: index, delimiter })
      index += delimiter.length
      start = index
      codeSeen = false
      words = new BodyWords()
      continue
    }
    index += 1
  }

  if (start < script.length) {
    bounds.push({ start, end: script.length, delimiter })
  }
  if (bounds.length === 0) {
    bounds.push({ start: 0, end: script.length, delimiter })
  }
  return bounds
}

/** The words the editor offers when the user asks for a completion. */
export interface CompletionItem {
  label: string
  detail: string
  insertText: string
  nameType: 'keyword' | 'database' | 'schema' | 'table' | 'column'
}

/** One relation the completion list knows about. */
export interface IndexedTable {
  name: string
  /** The database and the schema of the relation, joined by a full stop. */
  qualifier: string
}

/** One column the completion list knows about. */
export interface IndexedColumn {
  name: string
  /** The relation the column belongs to. */
  table: string
  /** The qualifier of that relation, which tells two relations apart. */
  qualifier: string
  dataType: string
}

/** The names the completion list draws from. */
export interface SchemaIndex {
  databases: string[]
  schemas: string[]
  tables: IndexedTable[]
  columns: IndexedColumn[]
}

/** Builds an empty index. */
export function emptySchemaIndex(): SchemaIndex {
  return { databases: [], schemas: [], tables: [], columns: [] }
}

/** The words that end the name of a relation in a FROM or a JOIN clause. */
const CLAUSE_WORDS = new Set([
  'AND',
  'CROSS',
  'DEFAULT',
  'EXCEPT',
  'FETCH',
  'FOR',
  'FULL',
  'GROUP',
  'HAVING',
  'INNER',
  'INTERSECT',
  'JOIN',
  'LATERAL',
  'LEFT',
  'LIMIT',
  'OFFSET',
  'ON',
  'OR',
  'ORDER',
  'OUTER',
  'RIGHT',
  'SELECT',
  'SET',
  'UNION',
  'USING',
  'VALUES',
  'WHERE',
  'WINDOW',
])

/** One word of a statement, with the quotes of the dialect removed. */
interface Token {
  /** The text of the word, without the quotes it carried. */
  text: string
  /** True when the word carried quotes, so it is a name and not a keyword. */
  quoted: boolean
}

/**
 * Splits a statement into words, names and single characters. The reader
 * steps over the comments and over the string literals, and it removes the
 * quotes of a name, so the caller reads a name as the user wrote it.
 */
function tokenize(statement: string, dialect: Dialect): Token[] {
  const tokens: Token[] = []
  const chars = [...statement]
  let index = 0

  const closingFor = (open: string): string => (open === '[' ? ']' : open)

  while (index < chars.length) {
    const character = chars[index] as string
    const next = chars[index + 1]

    if (opensDashComment(chars, index, dialect)) {
      while (index < chars.length && chars[index] !== '\n') {
        index += 1
      }
      continue
    }
    if (character === '/' && next === '*') {
      index += 2
      while (index < chars.length && !(chars[index] === '*' && chars[index + 1] === '/')) {
        index += 1
      }
      index += 2
      continue
    }
    if (character === "'") {
      index += 1
      while (index < chars.length && chars[index] !== "'") {
        index += 1
      }
      index += 1
      continue
    }
    if (
      character === '"' ||
      character === '`' ||
      (character === '[' && dialect === Dialect.MsSql)
    ) {
      const closing = closingFor(character)
      index += 1
      let name = ''
      while (index < chars.length && chars[index] !== closing) {
        name += chars[index]
        index += 1
      }
      index += 1
      tokens.push({ text: name, quoted: true })
      continue
    }
    if (/[A-Za-z0-9_$#@]/.test(character)) {
      let word = ''
      while (index < chars.length && /[A-Za-z0-9_$#@]/.test(chars[index] as string)) {
        word += chars[index]
        index += 1
      }
      tokens.push({ text: word, quoted: false })
      continue
    }
    if (!/\s/.test(character)) {
      tokens.push({ text: character, quoted: false })
    }
    index += 1
  }
  return tokens
}

/** True when a token ends the name of a relation. */
function endsTheName(token: Token): boolean {
  if (token.quoted) {
    return false
  }
  return CLAUSE_WORDS.has(token.text.toUpperCase()) || /^[(),;]$/.test(token.text)
}

/**
 * Reads the name that stands in front of the full stop at the cursor.
 *
 * `SELECT o.` gives `o`, and `SELECT [Sales].[dbo].` gives `dbo`, because the
 * name closest to the cursor is the one that decides the list. A cursor that
 * does not follow a full stop gives an empty text.
 */
export function qualifierBefore(text: string, offset: number): string {
  const position = Math.max(0, Math.min(offset, text.length))
  let head = text.slice(0, position)
  // The word the user is typing stands after the full stop, so it goes first.
  head = head.slice(0, head.length - wordBefore(head, head.length).length)
  if (!head.endsWith('.')) {
    return ''
  }
  head = head.slice(0, -1)

  const closing = head.endsWith(']')
    ? '['
    : head.endsWith('"')
      ? '"'
      : head.endsWith('`')
        ? '`'
        : ''
  if (closing !== '') {
    const start = head.lastIndexOf(closing, head.length - 2)
    return start < 0 ? '' : head.slice(start + 1, head.length - 1)
  }
  return wordBefore(head, head.length)
}

/** The words that a name of a relation follows. */
const RELATION_WORDS = new Set(['FROM', 'JOIN', 'UPDATE', 'INTO'])

/**
 * Reads the FROM clause and the JOIN clauses of a statement, and the target
 * of an UPDATE and of an INSERT INTO, and returns the relation that each
 * alias stands for. The name of a relation without an
 * alias is a key of its own, so `FROM Sales.dbo.Orders` answers for `Orders`
 * as well.
 */
export function tableAliases(statement: string, dialect: Dialect): Map<string, string> {
  const aliases = new Map<string, string>()
  const tokens = tokenize(statement, dialect)

  for (let index = 0; index < tokens.length; index += 1) {
    const word = tokens[index] as Token
    if (word.quoted) {
      continue
    }
    const upper = word.text.toUpperCase()
    if (!RELATION_WORDS.has(upper)) {
      continue
    }

    let cursor = index + 1
    // PostgreSQL writes ONLY in front of a relation to leave out its
    // children, and the word is no part of the name.
    const only = tokens[cursor]
    if (only && !only.quoted && only.text.toUpperCase() === 'ONLY') {
      cursor += 1
    }
    // One FROM clause can name more than one relation, with a comma between
    // two names, so the reader takes each name of the list.
    for (;;) {
      // The name of the relation, which can carry a database and a schema.
      const parts: string[] = []
      while (cursor < tokens.length && !endsTheName(tokens[cursor] as Token)) {
        const part = tokens[cursor] as Token
        if (part.text === '.') {
          cursor += 1
          continue
        }
        if (parts.length > 0 && (tokens[cursor - 1] as Token).text !== '.') {
          break
        }
        parts.push(part.text)
        cursor += 1
      }
      if (parts.length === 0) {
        break
      }
      const relation = parts[parts.length - 1] as string
      aliases.set(relation.toLowerCase(), relation)

      // The alias, with or without the word AS in front of it.
      let alias = tokens[cursor] as Token | undefined
      if (alias && !alias.quoted && alias.text.toUpperCase() === 'AS') {
        cursor += 1
        alias = tokens[cursor] as Token | undefined
      }
      // An alias is a name. A sign such as the = of `UPDATE a = 1` in the
      // ON DUPLICATE KEY clause of MySQL is not one.
      if (alias && !endsTheName(alias) && (alias.quoted || /^[A-Za-z_#@]/.test(alias.text))) {
        aliases.set(alias.text.toLowerCase(), relation)
        cursor += 1
      }
      if ((tokens[cursor] as Token | undefined)?.text !== ',') {
        break
      }
      cursor += 1
    }
    // The loop steps forward by one, and the word at the cursor may itself
    // start the next clause, so the cursor goes back by one here.
    index = Math.max(index, cursor - 1)
  }
  return aliases
}

/** What the statement around the cursor tells the completion list. */
export interface CompletionContext {
  /** The name in front of the full stop at the cursor, when there is one. */
  qualifier?: string
  /** The relation each alias of the statement stands for. */
  aliases?: Map<string, string>
  /**
   * The largest number of names to build. The editor shows a list of a few
   * rows and filters what it holds, so a schema of many thousand columns
   * needs no list of that size on each keystroke.
   */
  limit?: number
}

/**
 * Builds the list of completions for a prefix. The names of the objects
 * come first, because a name is what the user usually wants; keywords
 * follow.
 *
 * A qualifier gives the columns of the relation it names and nothing else. A
 * statement without a qualifier puts the columns of its own relations in
 * front of the other names.
 */
export function completionsFor(
  prefix: string,
  index: SchemaIndex,
  dialect: Dialect,
  context: CompletionContext = {},
): CompletionItem[] {
  const lower = prefix.toLowerCase()
  const matches = (name: string) => lower === '' || name.toLowerCase().startsWith(lower)
  const aliases = context.aliases ?? new Map<string, string>()
  const limit = context.limit ?? Number.POSITIVE_INFINITY

  const items: CompletionItem[] = []
  /** True while the list has room for another name. */
  const room = () => items.length < limit

  const qualifier = (context.qualifier ?? '').trim()
  if (qualifier !== '') {
    // The qualifier names an alias, a relation, a schema or a database.
    const relation = aliases.get(qualifier.toLowerCase()) ?? qualifier
    const wanted = relation.toLowerCase()
    for (const column of index.columns) {
      if (!room()) {
        break
      }
      const place = column.qualifier.toLowerCase().split('.')
      if (
        matches(column.name) &&
        (column.table.toLowerCase() === wanted || place.includes(wanted))
      ) {
        items.push({
          label: column.name,
          detail: `${column.dataType} in ${column.table}`,
          insertText: quoteIfNeeded(column.name, dialect),
          nameType: 'column',
        })
      }
    }
    if (items.length > 0) {
      return items
    }
    // The qualifier names a database or a schema whose columns are not held,
    // so the relations of that place are offered instead.
    for (const table of index.tables) {
      if (!room()) {
        break
      }
      if (matches(table.name) && table.qualifier.toLowerCase().split('.').includes(wanted)) {
        items.push({
          label: table.name,
          detail: table.qualifier,
          insertText: quoteIfNeeded(table.name, dialect),
          nameType: 'table',
        })
      }
    }
    return items
  }

  // The columns of the relations of the statement come first. The two
  // groups come from two walks of the same list, so no copy of the columns
  // of the whole schema is built.
  const inStatement = (column: IndexedColumn) => aliases.has(column.table.toLowerCase())
  for (const first of [true, false]) {
    for (const column of index.columns) {
      if (!room()) {
        break
      }
      if (inStatement(column) === first && matches(column.name)) {
        items.push({
          label: column.name,
          detail: `${column.dataType} in ${column.table}`,
          insertText: quoteIfNeeded(column.name, dialect),
          nameType: 'column',
        })
      }
    }
  }
  for (const table of index.tables) {
    if (!room()) {
      break
    }
    if (matches(table.name)) {
      items.push({
        label: table.name,
        detail: table.qualifier,
        insertText: quoteIfNeeded(table.name, dialect),
        nameType: 'table',
      })
    }
  }
  for (const schema of index.schemas) {
    if (!room()) {
      break
    }
    if (matches(schema)) {
      items.push({
        label: schema,
        detail: 'schema',
        insertText: quoteIfNeeded(schema, dialect),
        nameType: 'schema',
      })
    }
  }
  for (const database of index.databases) {
    if (!room()) {
      break
    }
    if (matches(database)) {
      items.push({
        label: database,
        detail: 'database',
        insertText: quoteIfNeeded(database, dialect),
        nameType: 'database',
      })
    }
  }
  for (const keyword of SQL_KEYWORDS) {
    if (!room()) {
      break
    }
    if (matches(keyword)) {
      items.push({
        label: keyword,
        detail: 'keyword',
        insertText: keyword,
        nameType: 'keyword',
      })
    }
  }
  return items
}

/**
 * Maps the dialect of the connection onto the dialect of the formatter.
 * Athena runs the Trino engine, so it takes the Trino rules.
 */
export function formatterDialect(dialect: Dialect): SqlLanguage {
  switch (dialect) {
    case Dialect.MsSql:
      return 'transactsql'
    case Dialect.MySql:
      return 'mysql'
    case Dialect.Postgres:
      return 'postgresql'
    case Dialect.Sqlite:
      return 'sqlite'
    default:
      return 'trino'
  }
}

/**
 * Lays out a statement in the style of the dialect. The call throws when the
 * text holds something the formatter cannot read, so the caller must catch.
 *
 * This function is the only place that knows the formatter package. A later
 * change of package therefore touches this file alone.
 */
export function formatSql(text: string, dialect: Dialect): string {
  return layOutStatement(text, {
    language: formatterDialect(dialect),
    // The editor indents with two spaces.
    tabWidth: 2,
    keywordCase: 'upper',
  })
}

/** Reads the word that stands just before the given position. */
export function wordBefore(text: string, offset: number): string {
  const position = Math.max(0, Math.min(offset, text.length))
  const head = text.slice(0, position)
  // The pattern matches an empty run, so the search always finds a start.
  return head.slice(head.search(/[A-Za-z0-9_]*$/))
}
