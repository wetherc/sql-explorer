import type { CellValue, Dialect, ResultSet } from '@/types/api'
import { formatCell, isNullCell } from './format'
import { quoteIdentifier } from './sql'

/**
 * Writes one field of a comma separated file. A field that holds a comma,
 * a quote, a line break or leading blank space is wrapped in quotes, and a
 * quote inside it is doubled.
 */
export function toCsvField(value: CellValue): string {
  if (isNullCell(value)) {
    return ''
  }
  const text = formatCell(value)
  const needsQuotes = /[",\r\n]/.test(text) || text !== text.trim()
  return needsQuotes ? `"${text.replace(/"/g, '""')}"` : text
}

/** Writes a whole result set as a comma separated file. */
export function toCsv(result: ResultSet, includeHeader = true): string {
  const lines: string[] = []
  if (includeHeader) {
    lines.push(result.columns.map((column) => toCsvField(column.name)).join(','))
  }
  for (const row of result.rows) {
    lines.push(row.map(toCsvField).join(','))
  }
  return lines.join('\n')
}

/**
 * Writes a whole result set as JSON. Each row becomes an object. A column
 * name that repeats gets a number after it, so that no value is lost.
 */
export function toJson(result: ResultSet, indent = 2): string {
  const names = uniqueColumnNames(result.columns.map((column) => column.name))
  const objects = result.rows.map((row) => {
    const object: Record<string, CellValue> = {}
    names.forEach((name, index) => {
      object[name] = row[index] ?? null
    })
    return object
  })
  return JSON.stringify(objects, null, indent)
}

/**
 * Writes a result set as a table of Markdown. A vertical bar inside a value
 * is escaped, and a line break becomes a space, because a cell of Markdown
 * holds one line.
 */
export function toMarkdown(result: ResultSet): string {
  const cell = (value: CellValue): string =>
    (isNullCell(value) ? '' : formatCell(value)).replace(/\|/g, '\\|').replace(/\r?\n/g, ' ')
  const lines = [
    `| ${result.columns.map((column) => cell(column.name)).join(' | ')} |`,
    `| ${result.columns.map(() => '---').join(' | ')} |`,
  ]
  for (const row of result.rows) {
    lines.push(`| ${row.map(cell).join(' | ')} |`)
  }
  return lines.join('\n')
}

/**
 * Writes a value as a literal of SQL. A number and a boolean go in as they
 * are, and everything else becomes a text with its quotes doubled. A value
 * that holds no data becomes NULL.
 */
export function toSqlLiteral(value: CellValue): string {
  if (isNullCell(value)) {
    return 'NULL'
  }
  if (typeof value === 'number') {
    return Number.isFinite(value) ? String(value) : 'NULL'
  }
  if (typeof value === 'boolean') {
    return value ? '1' : '0'
  }
  return `'${formatCell(value).replace(/'/g, "''")}'`
}

/**
 * Writes a result set as one INSERT statement for each row. Every name
 * carries the quotes of the dialect, because a column of a result can hold a
 * word that the engine reserves.
 */
export function toInsertStatements(result: ResultSet, table: string, dialect: Dialect): string {
  const columns = result.columns.map((column) => quoteIdentifier(column.name, dialect)).join(', ')
  const target = table
    .split('.')
    .map((part) => quoteIdentifier(part, dialect))
    .join('.')
  return result.rows
    .map((row) => {
      const values = result.columns.map((_, index) => toSqlLiteral(row[index] ?? null)).join(', ')
      return `INSERT INTO ${target} (${columns}) VALUES (${values});`
    })
    .join('\n')
}

/** Writes the selected cells as text that a spreadsheet accepts. */
export function toTabSeparated(rows: CellValue[][]): string {
  return rows
    .map((row) => row.map((value) => (isNullCell(value) ? '' : formatCell(value))).join('\t'))
    .join('\n')
}

/**
 * Makes every name in the list different from the others. A repeated name
 * gets a number, and the number rises until the name is free.
 */
export function uniqueColumnNames(names: string[]): string[] {
  const seen = new Map<string, number>()
  return names.map((name) => {
    const base = name === '' ? 'column' : name
    const count = seen.get(base) ?? 0
    seen.set(base, count + 1)
    if (count === 0) {
      return base
    }
    let candidate = `${base}_${count + 1}`
    let extra = count + 1
    while (seen.has(candidate)) {
      extra += 1
      candidate = `${base}_${extra}`
    }
    seen.set(candidate, 1)
    return candidate
  })
}

/** Writes a set of statements as one script. */
export function toScript(statements: string[]): string {
  return statements
    .map((statement) => statement.trim())
    .filter((statement) => statement.length > 0)
    .map((statement) => (statement.endsWith(';') ? statement : `${statement};`))
    .join('\n\n')
}

/** Builds the name of the file an export writes. */
export function exportFileName(base: string, extension: string, at = new Date()): string {
  const stamp = [
    at.getFullYear(),
    String(at.getMonth() + 1).padStart(2, '0'),
    String(at.getDate()).padStart(2, '0'),
    '-',
    String(at.getHours()).padStart(2, '0'),
    String(at.getMinutes()).padStart(2, '0'),
    String(at.getSeconds()).padStart(2, '0'),
  ].join('')
  const safeBase = base.replace(/[^A-Za-z0-9_-]+/g, '_').replace(/^_+|_+$/g, '') || 'result'
  return `${safeBase}-${stamp}.${extension}`
}
