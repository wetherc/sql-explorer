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

/** The names the completion list draws from. */
export interface SchemaIndex {
  databases: string[]
  schemas: string[]
  tables: Array<{ name: string; qualifier: string }>
  columns: Array<{ name: string; table: string; dataType: string }>
}

/** Builds an empty index. */
export function emptySchemaIndex(): SchemaIndex {
  return { databases: [], schemas: [], tables: [], columns: [] }
}

/**
 * Builds the list of completions for a prefix. The names of the objects
 * come first, because a name is what the user usually wants; keywords
 * follow.
 */
export function completionsFor(
  prefix: string,
  index: SchemaIndex,
  dialect: Dialect,
): CompletionItem[] {
  const lower = prefix.toLowerCase()
  const matches = (name: string) => lower === '' || name.toLowerCase().startsWith(lower)

  const items: CompletionItem[] = []

  for (const column of index.columns) {
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
