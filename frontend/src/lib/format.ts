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

/** Writes a moment as a local time of day, for the title of a kept result. */
export function formatClockTime(milliseconds: number): string {
  return new Date(milliseconds).toLocaleTimeString()
}

/** Shortens a statement to one line, for a list of past statements. */
export function summariseQuery(query: string, limit = 90): string {
  const oneLine = query.replace(/\s+/g, ' ').trim()
  return truncate(oneLine, limit)
}
