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

/** Quotes a name only when it needs quotes. */
export function quoteIfNeeded(name: string, dialect: Dialect): string {
  return isPlainIdentifier(name) ? name : quoteIdentifier(name, dialect)
}

/**
 * Returns the statement that surrounds the given position. The editor uses
 * it to run the statement under the cursor when nothing is selected.
 *
 * The split is a simple one that respects single quotes, double quotes and
 * comments. The backend splits again before it sends anything to a server,
 * so this only has to be good enough to pick the right block.
 */
export function statementAt(script: string, offset: number): string {
  const bounds = statementBounds(script)
  const position = Math.max(0, Math.min(offset, script.length))
  for (const [start, end] of bounds) {
    if (position >= start && position <= end) {
      return script.slice(start, end).trim()
    }
  }
  return script.trim()
}

/** Returns the start and the end of every statement in the script. */
export function statementBounds(script: string): Array<[number, number]> {
  const bounds: Array<[number, number]> = []
  let start = 0
  let index = 0
  let quote: string | null = null
  let lineComment = false
  let blockComment = false

  while (index < script.length) {
    const character = script[index]
    const next = script[index + 1]

    if (lineComment) {
      if (character === '\n') {
        lineComment = false
      }
      index += 1
      continue
    }
    if (blockComment) {
      if (character === '*' && next === '/') {
        blockComment = false
        index += 2
        continue
      }
      index += 1
      continue
    }
    if (quote) {
      if (character === quote) {
        if (next === quote) {
          index += 2
          continue
        }
        quote = null
      }
      index += 1
      continue
    }
    if (character === '-' && next === '-') {
      lineComment = true
      index += 2
      continue
    }
    if (character === '/' && next === '*') {
      blockComment = true
      index += 2
      continue
    }
    if (character === "'" || character === '"' || character === '`') {
      quote = character
      index += 1
      continue
    }
    if (character === ';') {
      bounds.push([start, index])
      start = index + 1
    }
    index += 1
  }

  if (start < script.length) {
    bounds.push([start, script.length])
  }
  if (bounds.length === 0) {
    bounds.push([0, script.length])
  }
  return bounds
}

/** The words the editor offers when the user asks for a completion. */
export interface CompletionItem {
  label: string
  detail: string
  insertText: string
  kind: 'keyword' | 'database' | 'schema' | 'table' | 'column'
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

    if (character === '-' && next === '-') {
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

/**
 * Reads the FROM clause and the JOIN clauses of a statement and returns the
 * relation that each alias stands for. The name of a relation without an
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
    if (upper !== 'FROM' && upper !== 'JOIN') {
      continue
    }

    // The name of the relation, which can carry a database and a schema.
    const parts: string[] = []
    let cursor = index + 1
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
      continue
    }
    const relation = parts[parts.length - 1] as string
    aliases.set(relation.toLowerCase(), relation)

    // The alias, with or without the word AS in front of it.
    let alias = tokens[cursor] as Token | undefined
    if (alias && !alias.quoted && alias.text.toUpperCase() === 'AS') {
      cursor += 1
      alias = tokens[cursor] as Token | undefined
    }
    if (alias && !endsTheName(alias)) {
      aliases.set(alias.text.toLowerCase(), relation)
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

  const items: CompletionItem[] = []

  const qualifier = (context.qualifier ?? '').trim()
  if (qualifier !== '') {
    // The qualifier names an alias, a relation, a schema or a database.
    const relation = aliases.get(qualifier.toLowerCase()) ?? qualifier
    const wanted = relation.toLowerCase()
    for (const column of index.columns) {
      const place = column.qualifier.toLowerCase().split('.')
      if (
        matches(column.name) &&
        (column.table.toLowerCase() === wanted || place.includes(wanted))
      ) {
        items.push({
          label: column.name,
          detail: `${column.dataType} in ${column.table}`,
          insertText: quoteIfNeeded(column.name, dialect),
          kind: 'column',
        })
      }
    }
    if (items.length > 0) {
      return items
    }
    // The qualifier names a database or a schema whose columns are not held,
    // so the relations of that place are offered instead.
    for (const table of index.tables) {
      if (matches(table.name) && table.qualifier.toLowerCase().split('.').includes(wanted)) {
        items.push({
          label: table.name,
          detail: table.qualifier,
          insertText: quoteIfNeeded(table.name, dialect),
          kind: 'table',
        })
      }
    }
    return items
  }

  // The columns of the relations of the statement come first.
  const inStatement = (column: IndexedColumn) => aliases.has(column.table.toLowerCase())
  const columns = [
    ...index.columns.filter(inStatement),
    ...index.columns.filter((column) => !inStatement(column)),
  ]
  for (const column of columns) {
    if (matches(column.name)) {
      items.push({
        label: column.name,
        detail: `${column.dataType} in ${column.table}`,
        insertText: quoteIfNeeded(column.name, dialect),
        kind: 'column',
      })
    }
  }
  for (const table of index.tables) {
    if (matches(table.name)) {
      items.push({
        label: table.name,
        detail: table.qualifier,
        insertText: quoteIfNeeded(table.name, dialect),
        kind: 'table',
      })
    }
  }
  for (const schema of index.schemas) {
    if (matches(schema)) {
      items.push({
        label: schema,
        detail: 'schema',
        insertText: quoteIfNeeded(schema, dialect),
        kind: 'schema',
      })
    }
  }
  for (const database of index.databases) {
    if (matches(database)) {
      items.push({
        label: database,
        detail: 'database',
        insertText: quoteIfNeeded(database, dialect),
        kind: 'database',
      })
    }
  }
  for (const keyword of SQL_KEYWORDS) {
    if (matches(keyword)) {
      items.push({
        label: keyword,
        detail: 'keyword',
        insertText: keyword,
        kind: 'keyword',
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
