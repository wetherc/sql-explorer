/**
 * Writes an XLSX file. An XLSX file is a ZIP container that holds a few XML
 * parts, so `fflate` builds the container and this file builds the parts.
 * The application therefore needs no large spreadsheet library.
 *
 * Every text goes in the sheet as an inline string. That makes the file
 * larger than a shared table of strings would, and it keeps the writer to
 * one pass over the rows.
 */
import { zipSync, strToU8 } from 'fflate'
import { ErrorCategory, type CellValue, type ErrorPayload, type ResultSet } from '@/types/api'
import { formatCell, isNullCell, isPlainNumber } from './format'

/** Escapes the five characters that XML reserves. */
export function escapeXml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;')
}

/**
 * Drops the characters that XML 1.0 forbids. A database can hold such a
 * character, and a spreadsheet refuses to open a file that carries one.
 */
export function stripForbiddenXml(text: string): string {
  // eslint-disable-next-line no-control-regex
  return text.replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\uFFFE\uFFFF]/g, '')
}

/** The largest number of columns that a sheet can contain. The last is XFD. */
export const MAX_SHEET_COLUMNS = 16384

/** The largest number of significant digits that Excel keeps in a number. */
const EXCEL_DIGITS = 15

/**
 * The largest number of characters, counted in UTF-16 units, that Excel
 * accepts in one cell. A longer text makes Excel repair the file.
 */
export const MAX_CELL_UNITS = 32767

/**
 * The text of a number cell when Excel can keep the value exactly: a decimal
 * number with at most 15 significant digits whose value is finite. A longer
 * number goes in as text, because Excel would round 1234567890123456789 to
 * 1234567890123456800.
 */
function excelNumber(text: string): string | null {
  if (!isPlainNumber(text)) {
    return null
  }
  const digits = text
    .split(/[eE]/)[0]!
    .replace(/^[+-]/, '')
    .replace('.', '')
    .replace(/^0+|0+$/g, '')
  const value = Number(text)
  return Number.isFinite(value) && digits.length <= EXCEL_DIGITS ? String(value) : null
}

/** Cuts a text to the number of characters that one cell accepts. */
export function cellText(text: string): string {
  if (text.length <= MAX_CELL_UNITS) {
    return text
  }
  // A cut between the two halves of a surrogate pair would leave half a
  // character.
  const last = text.charCodeAt(MAX_CELL_UNITS - 1)
  const end = last >= 0xd800 && last <= 0xdbff ? MAX_CELL_UNITS - 1 : MAX_CELL_UNITS
  return text.slice(0, end)
}

/** Writes a cell that holds a text. */
function textCell(reference: string, text: string): string {
  const escaped = escapeXml(cellText(stripForbiddenXml(text)))
  return `<c r="${reference}" t="inlineStr"><is><t xml:space="preserve">${escaped}</t></is></c>`
}

/** Names a column of a spreadsheet: 1 gives A, 27 gives AA. */
export function columnName(index: number): string {
  let rest = index
  let name = ''
  while (rest > 0) {
    const remainder = (rest - 1) % 26
    name = String.fromCharCode(65 + remainder) + name
    rest = Math.floor((rest - remainder - 1) / 26)
  }
  return name
}

/**
 * Writes one cell of the sheet. A number, and a text that holds only a
 * number, go in as a number when Excel can keep the value exactly, so that
 * `SUM` reads a DECIMAL column and every PostgreSQL column of the simple
 * protocol. Any other number goes in as text.
 */
function cellXml(reference: string, value: CellValue): string {
  if (isNullCell(value)) {
    return ''
  }
  if (typeof value === 'boolean') {
    return `<c r="${reference}" t="b"><v>${value ? 1 : 0}</v></c>`
  }
  const text = formatCell(value)
  const number = typeof value === 'number' || typeof value === 'string' ? excelNumber(text) : null
  return number === null ? textCell(reference, text) : `<c r="${reference}"><v>${number}</v></c>`
}

/** Writes one row of the sheet. */
function rowXml(values: CellValue[], rowNumber: number): string {
  const cells = values
    .map((value, index) => cellXml(`${columnName(index + 1)}${rowNumber}`, value))
    .join('')
  return `<row r="${rowNumber}">${cells}</row>`
}

/** Writes the sheet part, with the column names on the first row. */
export function sheetXml(result: ResultSet): string {
  // Each name goes in as a text, so a column named 2024 keeps its name.
  const names = result.columns.map((column, index) =>
    textCell(`${columnName(index + 1)}1`, column.name),
  )
  const header = `<row r="1">${names.join('')}</row>`
  const body = result.rows.map((row, index) => rowXml(row, index + 2)).join('')
  return (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">' +
    `<sheetData>${header}${body}</sheetData>` +
    '</worksheet>'
  )
}

const CONTENT_TYPES =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
  '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
  '<Default Extension="xml" ContentType="application/xml"/>' +
  '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>' +
  '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' +
  '</Types>'

const ROOT_RELATIONSHIPS =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
  '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>' +
  '</Relationships>'

const WORKBOOK_RELATIONSHIPS =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
  '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>' +
  '</Relationships>'

/** Writes the workbook part, which names the one sheet of the file. */
export function workbookXml(sheetName: string): string {
  return (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" ' +
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">' +
    `<sheets><sheet name="${escapeXml(sheetName)}" sheetId="1" r:id="rId1"/></sheets>` +
    '</workbook>'
  )
}

/**
 * Cleans a name for a sheet. A sheet name holds at most 31 characters and
 * none of the characters that Excel reserves. Excel also refuses an
 * apostrophe at the start or the end of the name, and it keeps the name
 * `History` for a sheet of its own. Excel repairs a file that breaks one of
 * these rules.
 */
export function sheetName(name: string): string {
  const cleaned = name.replace(/[\\/?*[\]:]/g, '_').trim()
  // The backend counts code points, so a character outside the basic plane
  // counts as one character here too.
  const chars = Array.from(cleaned === '' ? 'Result' : cleaned).slice(0, 31)
  // The cut to 31 characters can put an apostrophe at the end, so the ends
  // are read after the cut.
  for (const at of [0, chars.length - 1]) {
    if (chars[at] === "'") {
      chars[at] = '_'
    }
  }
  const joined = chars.join('')
  return joined.toLowerCase() === 'history' ? `${joined}_` : joined
}

/**
 * Builds a whole XLSX file from one result set.
 *
 * A result with more columns than a sheet can contain gives the error that
 * the backend gives for the same result. Excel repairs a file with a column
 * past XFD, and a cut of the columns would drop data with no sign of it in
 * the file.
 */
export function toXlsx(result: ResultSet, name = 'Result'): Uint8Array {
  const count = result.columns.length
  if (count > MAX_SHEET_COLUMNS) {
    const payload: ErrorPayload = {
      category: ErrorCategory.Unsupported,
      message: `Excel sheets allow at most ${MAX_SHEET_COLUMNS} columns, but this result has ${count}. Export it as CSV or JSON instead.`,
      detail: null,
    }
    // The error is also a payload, so the notice shows the category of the fault.
    throw Object.assign(new Error(payload.message), payload)
  }
  return zipSync({
    '[Content_Types].xml': strToU8(CONTENT_TYPES),
    '_rels/.rels': strToU8(ROOT_RELATIONSHIPS),
    'xl/workbook.xml': strToU8(workbookXml(sheetName(name))),
    'xl/_rels/workbook.xml.rels': strToU8(WORKBOOK_RELATIONSHIPS),
    'xl/worksheets/sheet1.xml': strToU8(sheetXml(result)),
  })
}

/**
 * Writes bytes as base64 text. The bytes travel to the backend that way,
 * because the raw form of the bridge carries one body and no path beside it.
 */
export function bytesToBase64(bytes: Uint8Array): string {
  let text = ''
  const step = 0x8000
  for (let start = 0; start < bytes.length; start += step) {
    const part = bytes.subarray(start, start + step)
    text += String.fromCharCode(...part)
  }
  return btoa(text)
}
