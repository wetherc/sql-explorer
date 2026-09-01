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
 * The value a sort compares. A cell without a value gives null, a number
 * stays a number, and every other cell gives its text. One collator serves
 * every comparison, because a new collator for each pair costs more than
 * the comparison itself.
 */
export type SortKey = number | string | null

const collator = new Intl.Collator(undefined, { numeric: true })

/**
 * Builds the value a sort compares from one cell. A sort of many rows builds
 * one key for each row and then compares the keys, so the text of a cell is
 * built once and not once for each comparison.
 */
export function sortKey(value: CellValue): SortKey {
  if (isNullCell(value)) {
    return null
  }
  return typeof value === 'number' ? value : formatCell(value)
}

/**
 * Compares two sort keys. Keys without a value go to the end, two numbers
 * compare as numbers, and every other pair compares as text.
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
  return collator.compare(String(left), String(right))
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
