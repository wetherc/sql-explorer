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
 * Compares two cells for the sort of the grid. Cells without a value go to
 * the end, numbers compare as numbers, and everything else compares as
 * text.
 */
export function compareCells(left: CellValue, right: CellValue): number {
  const leftEmpty = isNullCell(left)
  const rightEmpty = isNullCell(right)
  if (leftEmpty && rightEmpty) {
    return 0
  }
  if (leftEmpty) {
    return 1
  }
  if (rightEmpty) {
    return -1
  }
  if (typeof left === 'number' && typeof right === 'number') {
    return left - right
  }
  if (typeof left === 'boolean' && typeof right === 'boolean') {
    return Number(left) - Number(right)
  }
  return formatCell(left).localeCompare(formatCell(right), undefined, { numeric: true })
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
