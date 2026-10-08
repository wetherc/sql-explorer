import { formatRowCount } from '@/lib/format'

/** What a run to a file wrote for the result that the grid shows. */
export interface SavedFile {
  path: string
  /** The rows that the file received. */
  rows: number
  /** True when the export row limit, or the room of an Excel sheet,
   *  stopped the file. */
  truncated: boolean
}

/** The sentence that names the rows of the file. */
function savedRows(file: SavedFile): string {
  if (file.truncated) {
    const verb = file.rows === 1 ? 'was' : 'were'
    return `The first ${formatRowCount(file.rows)} ${verb} saved to ${file.path}.`
  }
  return file.rows === 1
    ? `The row was saved to ${file.path}.`
    : `All ${file.rows.toLocaleString()} rows were saved to ${file.path}.`
}

/**
 * The note above the grid of a result whose rows also went to a file. The
 * grid shows the first rows, up to the row limit, and the file has the rest.
 */
export function savedFileNote(shown: number, cut: boolean, file: SavedFile): string {
  const saved = savedRows(file)
  return cut ? `Showing the first ${formatRowCount(shown)}. ${saved}` : saved
}
