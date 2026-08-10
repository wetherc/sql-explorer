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
import type { CellValue, ResultSet } from '@/types/api'
import { formatCell, isNullCell } from './format'

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
  return text.replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F]/g, '')
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

/** Writes one cell of the sheet. */
function cellXml(reference: string, value: CellValue): string {
  if (isNullCell(value)) {
    return ''
  }
  if (typeof value === 'number' && Number.isFinite(value)) {
    return `<c r="${reference}"><v>${value}</v></c>`
  }
  if (typeof value === 'boolean') {
    return `<c r="${reference}" t="b"><v>${value ? 1 : 0}</v></c>`
  }
  const text = escapeXml(stripForbiddenXml(formatCell(value)))
  return `<c r="${reference}" t="inlineStr"><is><t xml:space="preserve">${text}</t></is></c>`
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
  const header = rowXml(
    result.columns.map((column) => column.name),
    1,
  )
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
 * none of the characters that Excel reserves.
 */
export function sheetName(name: string): string {
  const cleaned = name.replace(/[\\/?*[\]:]/g, '_').trim()
  return (cleaned === '' ? 'Result' : cleaned).slice(0, 31)
}

/** Builds a whole XLSX file from one result set. */
export function toXlsx(result: ResultSet, name = 'Result'): Uint8Array {
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
