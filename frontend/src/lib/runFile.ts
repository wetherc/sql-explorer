import { formatRowCount } from '@/lib/format'
import type { RunFileFormat, RunFileSummary, SavedSet } from '@/types/api'

/** What a run to a file wrote for the result that the grid shows. */
export interface SavedFile {
  path: string
  /** The rows that the file received. */
  rows: number
  /** True when the export row limit, or the room of an Excel sheet,
   *  stopped the file. */
  truncated: boolean
  /** The sheet of the result, in an Excel file with a sheet for each
   *  result. */
  sheet?: string
}

/** The file of one saved result set, for the note above its grid. */
export function savedFile(set: SavedSet): SavedFile {
  const file: SavedFile = { path: set.path, rows: set.rows, truncated: set.truncated }
  if (set.sheet) {
    file.sheet = set.sheet
  }
  return file
}

/** The sentence that names the rows of the file. */
function savedRows(file: SavedFile): string {
  const target = file.sheet ? `the "${file.sheet}" sheet of ${file.path}` : file.path
  if (file.truncated) {
    const verb = file.rows === 1 ? 'was' : 'were'
    return `The first ${formatRowCount(file.rows)} ${verb} saved to ${target}.`
  }
  return file.rows === 1
    ? `The row was saved to ${target}.`
    : `All ${file.rows.toLocaleString()} rows were saved to ${target}.`
}

/**
 * The note above the grid of a result whose rows also went to a file. The
 * grid shows the first rows, up to the row limit, and the file has the rest.
 */
export function savedFileNote(shown: number, cut: boolean, file: SavedFile): string {
  const saved = savedRows(file)
  return cut ? `Showing the first ${formatRowCount(shown)}. ${saved}` : saved
}

/** Where a run to a file sends the result sets after the first. */
export type SetChoice = 'first' | 'each'

/** The key of the last choice in the browser store. */
export const SET_CHOICE_KEY = 'sql-explorer.runFileSets'

/** The choice the user made the last time a script gave several results. */
export function loadSetChoice(): SetChoice {
  try {
    return localStorage.getItem(SET_CHOICE_KEY) === 'each' ? 'each' : 'first'
  } catch {
    // A store that refuses the read gives the choice that writes one file.
    return 'first'
  }
}

/** Keeps the choice for the next run. */
export function saveSetChoice(choice: SetChoice): void {
  try {
    localStorage.setItem(SET_CHOICE_KEY, choice)
  } catch {
    // A store that refuses the write forgets the choice, and the next run
    // asks with the first choice selected.
  }
}

/** The label of the choice that saves every result set. */
export function eachSetLabel(format: RunFileFormat): string {
  return format === 'xlsx' ? 'One sheet per result set' : 'One file per result set'
}

/** The name of the file of the second result set beside the chosen file:
 *  `orders-2.csv` beside `/a/orders.csv`. */
export function secondFileName(path: string): string {
  const name = path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1)
  const dot = name.lastIndexOf('.')
  return dot > 0 ? `${name.slice(0, dot)}-2${name.slice(dot)}` : `${name}-2`
}

/**
 * The words of the notice for a run that saved more than one result set, or
 * null for a run that saved one.
 */
export function savedSetsMessage(summary: RunFileSummary): string | null {
  const count = summary.sets.length
  if (count < 2) {
    return null
  }
  const rows = formatRowCount(summary.rows)
  return summary.sets[0]?.sheet
    ? `Saved ${rows} to ${count} sheets in ${summary.path}.`
    : `Saved ${rows} to ${count} files, starting with ${summary.path}.`
}
