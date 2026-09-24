import type { CellValue } from '@/types/api'

/** The text the grid shows for a cell that holds no value. */
export const NULL_TEXT = 'NULL'

/**
 * Renders one cell for the grid. A cell that holds no value gets its own
 * text, so that it is not confused with an empty string.
 */
export function formatCell(value: CellValue): string {
  if (value === null || value === undefined) {
    return NULL_TEXT
  }
  if (typeof value === 'string') {
    return value
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value)
  }
  return JSON.stringify(value)
}

/**
 * True when the whole text is a decimal number: an optional sign, digits
 * with at most one decimal point, and an optional exponent. A spreadsheet
 * reads such a text as a number.
 */
export function isPlainNumber(text: string): boolean {
  return /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/.test(text)
}

/** True when the cell holds no value. */
export function isNullCell(value: CellValue): boolean {
  return value === null || value === undefined
}

/** Cuts a long text and marks the cut, so that one cell keeps its row height. */
export function truncate(text: string, limit = 200): string {
  if (limit <= 0 || text.length <= limit) {
    return text
  }
  return `${text.slice(0, limit)}…`
}

/** Writes a length of time in the largest unit that keeps the number above one. */
export function formatDuration(milliseconds: number): string {
  if (milliseconds < 1000) {
    return `${Math.round(milliseconds)} ms`
  }
  const seconds = milliseconds / 1000
  if (seconds < 60) {
    return `${seconds.toFixed(2)} s`
  }
  const wholeMinutes = Math.floor(seconds / 60)
  const restSeconds = Math.round(seconds - wholeMinutes * 60)
  return `${wholeMinutes} min ${restSeconds} s`
}

/** Writes a count of rows with the correct singular or plural word. */
export function formatRowCount(count: number): string {
  return count === 1 ? '1 row' : `${count.toLocaleString()} rows`
}

/**
 * A decimal number that arrived as text, held so that two of them compare
 * exactly. `digits` holds the significant digits without leading and
 * trailing zeros, and `point` is the place of the decimal point among them,
 * so 0.05 gives the digits 5 and the point -1. Zero has no digits.
 */
export interface DecimalKey {
  text: string
  negative: boolean
  digits: string
  point: number
}

/**
 * The value a sort compares. A cell without a value gives null, a number
 * stays a number, a text that holds only a decimal number gives a decimal
 * key, and every other cell gives its text. One collator serves every
 * comparison, because a new collator for each pair costs more than the
 * comparison itself.
 */
export type SortKey = number | string | DecimalKey | null

const collator = new Intl.Collator(undefined, { numeric: true })

/** Reads a text that `isPlainNumber` accepts as a decimal key. */
function decimalKey(text: string): DecimalKey {
  const [, sign, whole, fraction, power] = /^([+-]?)(\d*)\.?(\d*)(?:[eE]([+-]?\d+))?$/.exec(text)!
  const all = `${whole}${fraction}`
  const trimmed = all.replace(/^0+/, '')
  const digits = trimmed.replace(/0+$/, '')
  const point = whole!.length + Number(power ?? 0) - (all.length - trimmed.length)
  return { text, negative: sign === '-' && digits !== '', digits, point }
}

/**
 * Builds the value a sort compares from one cell. A sort of many rows builds
 * one key for each row and then compares the keys, so the text of a cell is
 * built once and not once for each comparison.
 *
 * PostgreSQL values of the simple protocol and DECIMAL values arrive as
 * text. A collator compares runs of digits and ignores the sign, so it puts
 * -5.00 in front of -10.00 and 1.5 in front of 1.25. A decimal key compares
 * the value.
 */
export function sortKey(value: CellValue): SortKey {
  if (isNullCell(value)) {
    return null
  }
  if (typeof value === 'number') {
    return value
  }
  const text = formatCell(value)
  return typeof value === 'string' && isPlainNumber(text) ? decimalKey(text) : text
}

/** Compares the size of two decimal keys and ignores their signs. */
function compareMagnitudes(left: DecimalKey, right: DecimalKey): number {
  if (left.digits === '' || right.digits === '') {
    return Number(left.digits !== '') - Number(right.digits !== '')
  }
  if (left.point !== right.point) {
    return left.point < right.point ? -1 : 1
  }
  // Both digit strings start with a digit that is not zero at the same
  // place, so the order of the texts is the order of the values.
  return left.digits < right.digits ? -1 : left.digits > right.digits ? 1 : 0
}

/** Compares two decimal keys by value. */
function compareDecimals(left: DecimalKey, right: DecimalKey): number {
  if (left.negative !== right.negative) {
    return left.negative ? -1 : 1
  }
  const order = compareMagnitudes(left, right)
  // A subtraction gives 0 for two equal keys, where a negation gives -0.
  return left.negative ? 0 - order : order
}

/**
 * The decimal key of a key that holds a number, or null. A number that is
 * not finite has no decimal form.
 */
function asDecimal(key: SortKey): DecimalKey | null {
  if (typeof key === 'number') {
    return Number.isFinite(key) ? decimalKey(String(key)) : null
  }
  return typeof key === 'object' ? key : null
}

/** The text of a key, for a comparison as text. */
function keyText(key: Exclude<SortKey, null>): string {
  return typeof key === 'object' ? key.text : String(key)
}

/**
 * Compares two sort keys. Keys without a value go to the end, two numbers
 * compare as numbers, a number and a decimal key compare by value, and every
 * other pair compares as text.
 */
export function compareSortKeys(left: SortKey, right: SortKey): number {
  if (left === null && right === null) {
    return 0
  }
  if (left === null) {
    return 1
  }
  if (right === null) {
    return -1
  }
  if (typeof left === 'number' && typeof right === 'number') {
    return left - right
  }
  const leftDecimal = asDecimal(left)
  const rightDecimal = asDecimal(right)
  if (leftDecimal && rightDecimal) {
    return compareDecimals(leftDecimal, rightDecimal)
  }
  return collator.compare(keyText(left), keyText(right))
}

/**
 * Compares two cells for the sort of the grid. Cells without a value go to
 * the end, numbers compare as numbers, and everything else compares as
 * text.
 */
export function compareCells(left: CellValue, right: CellValue): number {
  return compareSortKeys(sortKey(left), sortKey(right))
}

/** Writes a moment as a short local date and time. */
export function formatTimestamp(value: string): string {
  const moment = new Date(value)
  if (Number.isNaN(moment.getTime())) {
    return value
  }
  return moment.toLocaleString()
}

/** The number of bytes in one terabyte, as a storage unit counts them. */
export const BYTES_IN_TERABYTE = 1024 ** 4

/** Writes a byte count in the largest unit that keeps the number above one. */
export function formatBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return unit === 0 ? `${Math.round(value)} B` : `${value.toFixed(2)} ${units[unit]}`
}

/**
 * Gives the price of a scan, from a rate for each terabyte. The figure is an
 * estimate, because the rate changes by region and by contract.
 */
export function scanCost(bytes: number, pricePerTerabyte: number): number {
  return (bytes / BYTES_IN_TERABYTE) * pricePerTerabyte
}

/**
 * Writes a price in US dollars. A price below one cent keeps four places, so
 * that a small scan does not read as nothing.
 */
export function formatCost(dollars: number): string {
  return dollars > 0 && dollars < 0.01 ? `$${dollars.toFixed(4)}` : `$${dollars.toFixed(2)}`
}

/** Writes a moment as a local time of day, for the title of a kept result. */
export function formatClockTime(milliseconds: number): string {
  return new Date(milliseconds).toLocaleTimeString()
}

/** Shortens a statement to one line, for a list of past statements. */
export function summariseQuery(query: string, limit = 90): string {
  const oneLine = query.replace(/\s+/g, ' ').trim()
  return truncate(oneLine, limit)
}

/**
 * Says what a close of one connection would stop. The question before a close
 * uses it, so the user reads the same words wherever the close begins.
 */
export function stoppedStatementsMessage(count: number): string {
  const head = count === 1 ? 'One statement is' : `${count} statements are`
  return `${head} running on this connection. Closing it stops them, and their rows are lost.`
}
