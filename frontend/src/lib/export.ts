import type { CellValue, Dialect, ResultSet } from '@/types/api'
import { formatCell, isNullCell, isPlainNumber } from './format'
import { quoteIdentifier } from './sql'

/**
 * The mark of the byte order that a comma separated file carries. Excel
 * reads a file without the mark in the code page of the system, and a
 * value outside ASCII then arrives damaged.
 */
export const CSV_BOM = '\ufeff'

/** The line end of a comma separated file, which Excel expects. */
const CSV_LINE_END = '\r\n'

/**
 * True when a spreadsheet would read the text as a formula. A cell that
 * begins with one of these marks runs as a formula in Excel, so the export
 * puts an apostrophe in front of it. A number that arrives as text, such as
 * a DECIMAL value or a PostgreSQL value of the simple protocol, keeps its
 * sign, because a spreadsheet reads `-5` as a number and an apostrophe would
 * stay in the value that a loader reads.
 */
export function startsAFormula(text: string): boolean {
  return text.length > 0 && '=+-@\t\r'.includes(text[0]!) && !isPlainNumber(text)
}

/**
 * Writes one field of a comma separated file. A field that holds a comma,
 * a quote, a line break or leading blank space is wrapped in quotes, and a
 * quote inside it is doubled.
 *
 * A text value that starts with a formula mark gets an apostrophe in front,
 * because a spreadsheet would otherwise run the value as a formula. The
 * apostrophe changes the exported text, and the safety of the reader weighs
 * more than the exact form of such a value. A number keeps its sign,
 * because a spreadsheet reads it as a number.
 */
export function toCsvField(value: CellValue): string {
  if (isNullCell(value)) {
    return ''
  }
  let text = formatCell(value)
  if (typeof value !== 'number' && startsAFormula(text)) {
    text = `'${text}`
  }
  const needsQuotes = /[",\r\n]/.test(text) || text !== text.trim()
  return needsQuotes ? `"${text.replace(/"/g, '""')}"` : text
}

/**
 * Writes a whole result set as a comma separated file. The text begins with
 * the mark of the byte order and ends each line with a carriage return and
 * a line feed, so Excel reads the file in UTF-8.
 */
export function toCsv(result: ResultSet, includeHeader = true): string {
  const lines: string[] = []
  if (includeHeader) {
    lines.push(result.columns.map((column) => toCsvField(column.name)).join(','))
  }
  for (const row of result.rows) {
    lines.push(row.map(toCsvField).join(','))
  }
  return CSV_BOM + lines.map((line) => `${line}${CSV_LINE_END}`).join('')
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

/**
 * Writes the selected cells as text that a spreadsheet accepts. A cell
 * without a value gets the same word the grid shows, so that it stays
 * different from an empty text.
 */
export function toTabSeparated(rows: CellValue[][]): string {
  return rows.map((row) => row.map((value) => formatCell(value)).join('\t')).join('\n')
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
