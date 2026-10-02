import { describe, expect, it } from 'vitest'
import { unzipSync, strFromU8 } from 'fflate'
import {
  bytesToBase64,
  cellText,
  columnName,
  MAX_CELL_UNITS,
  MAX_SHEET_COLUMNS,
  escapeXml,
  sheetName,
  sheetXml,
  stripForbiddenXml,
  toXlsx,
  workbookXml,
} from '@/lib/xlsx'
import { ErrorCategory, type CellValue, type ResultSet } from '@/types/api'

const result: ResultSet = {
  columns: [
    { name: 'id', typeName: 'int' },
    { name: 'name', typeName: 'text' },
    { name: 'ok', typeName: 'bit' },
  ],
  rows: [
    [1, 'Ada & Co', true],
    [2, null, false],
    [Number.POSITIVE_INFINITY, 'linebreak', null],
  ],
  truncated: false,
}

describe('escapeXml', () => {
  it('escapes the five characters that XML reserves', () => {
    expect(escapeXml('a&b<c>d"e\'f')).toBe('a&amp;b&lt;c&gt;d&quot;e&apos;f')
  })
})

describe('stripForbiddenXml', () => {
  it('drops a control character and keeps a tab and a line break', () => {
    expect(stripForbiddenXml('a\u0000b\u0007c\u001fd')).toBe('abcd')
    expect(stripForbiddenXml('a\tb\nc')).toBe('a\tb\nc')
    expect(stripForbiddenXml('a\uFFFEb\uFFFFc')).toBe('abc')
  })
})

describe('columnName', () => {
  it('names the columns of a spreadsheet', () => {
    expect(columnName(1)).toBe('A')
    expect(columnName(26)).toBe('Z')
    expect(columnName(27)).toBe('AA')
    expect(columnName(52)).toBe('AZ')
    expect(columnName(703)).toBe('AAA')
    expect(columnName(0)).toBe('')
  })
})

describe('sheetName', () => {
  it('drops the characters that a sheet name forbids', () => {
    expect(sheetName('a/b:c*d')).toBe('a_b_c_d')
  })

  it('falls back when the name holds nothing', () => {
    expect(sheetName('   ')).toBe('Result')
  })

  it('cuts a long name at thirty one characters', () => {
    expect(sheetName('x'.repeat(40))).toHaveLength(31)
  })

  it('counts a character outside the basic plane as one character', () => {
    expect(sheetName('😀'.repeat(40))).toBe('😀'.repeat(31))
  })

  it('puts no apostrophe at an end of the name', () => {
    expect(sheetName("'q'")).toBe('_q_')
    expect(sheetName("'")).toBe('_')
    expect(sheetName("it's")).toBe("it's")
    // The cut to 31 characters can leave an apostrophe at the end.
    expect(sheetName(`${'x'.repeat(30)}'abc`)).toBe(`${'x'.repeat(30)}_`)
  })

  it('gives no sheet the name that Excel keeps for its own sheet', () => {
    expect(sheetName('History')).toBe('History_')
    expect(sheetName(' hIsToRy ')).toBe('hIsToRy_')
    expect(sheetName('History 2')).toBe('History 2')
  })
})

describe('sheetXml', () => {
  it('writes the names on the first row and the rows below', () => {
    const xml = sheetXml(result)
    expect(xml).toContain('<row r="1">')
    expect(xml).toContain('<t xml:space="preserve">id</t>')
    expect(xml).toContain('<c r="A2"><v>1</v></c>')
    expect(xml).toContain('Ada &amp; Co')
  })

  it('writes a boolean as a boolean and leaves a cell without a value empty', () => {
    const xml = sheetXml(result)
    expect(xml).toContain('<c r="C2" t="b"><v>1</v></c>')
    expect(xml).toContain('<c r="C3" t="b"><v>0</v></c>')
    expect(xml).not.toContain('r="B3"')
  })

  it('writes a number that is not finite as a text', () => {
    const xml = sheetXml(result)
    expect(xml).toContain('<c r="A4" t="inlineStr">')
  })

  it('writes a number as a number when Excel keeps it exactly', () => {
    const cells: CellValue[] = [
      '-5',
      '-10.00',
      '+.5',
      '0.000123456789012345',
      1e21,
      '1234567890123456',
      '1e400',
      '12a',
      1234567890123456,
      [1],
    ]
    const xml = sheetXml({
      columns: cells.map((_cell, index) => ({ name: String(2024 + index), typeName: 'x' })),
      rows: [cells],
      truncated: false,
    })
    expect(xml).toContain('<c r="A1" t="inlineStr"><is><t xml:space="preserve">2024</t>')
    expect(xml).toContain('<c r="A2"><v>-5</v></c>')
    expect(xml).toContain('<c r="B2"><v>-10</v></c>')
    expect(xml).toContain('<c r="C2"><v>0.5</v></c>')
    expect(xml).toContain('<c r="D2"><v>0.000123456789012345</v></c>')
    expect(xml).toContain('<c r="E2"><v>1e+21</v></c>')
    for (const column of ['F', 'G', 'H', 'I', 'J']) {
      expect(xml).toContain(`<c r="${column}2" t="inlineStr">`)
    }
  })
})

describe('cellText', () => {
  it('cuts a long text to the bound of a cell', () => {
    expect(cellText('a'.repeat(MAX_CELL_UNITS + 5))).toHaveLength(MAX_CELL_UNITS)
    expect(cellText('short')).toBe('short')
    // A character outside the basic plane is never split.
    const wide = `${'a'.repeat(MAX_CELL_UNITS - 1)}\u{1F600}`
    expect(cellText(wide)).toBe('a'.repeat(MAX_CELL_UNITS - 1))
  })
})

describe('workbookXml', () => {
  it('names the one sheet of the file', () => {
    expect(workbookXml('Data & More')).toContain('name="Data &amp; More"')
  })
})

describe('toXlsx', () => {
  it('builds a container that holds every part an XLSX file needs', () => {
    const bytes = toXlsx(result, 'Query 1')
    const parts = unzipSync(bytes)
    expect(Object.keys(parts).sort()).toEqual([
      '[Content_Types].xml',
      '_rels/.rels',
      'xl/_rels/workbook.xml.rels',
      'xl/workbook.xml',
      'xl/worksheets/sheet1.xml',
    ])
    expect(strFromU8(parts['xl/workbook.xml']!)).toContain('name="Query 1"')
    expect(strFromU8(parts['xl/worksheets/sheet1.xml']!)).toContain('<row r="1">')
  })

  it('names the sheet Result when no name is given', () => {
    const parts = unzipSync(toXlsx(result))
    expect(strFromU8(parts['xl/workbook.xml']!)).toContain('name="Result"')
  })

  it('refuses more columns than a sheet can contain', () => {
    const wide = (count: number): ResultSet => ({
      columns: Array.from({ length: count }, (_, index) => ({
        name: String(index),
        typeName: 'int',
      })),
      rows: [],
      truncated: false,
    })
    let caught: unknown = null
    try {
      toXlsx(wide(MAX_SHEET_COLUMNS + 1))
    } catch (error) {
      caught = error
    }
    expect(caught).toBeInstanceOf(Error)
    expect(caught).toMatchObject({
      category: ErrorCategory.Unsupported,
      message: expect.stringContaining('16384'),
      detail: null,
    })
    expect((caught as Error).message).toContain('16385 columns')

    const parts = unzipSync(toXlsx(wide(MAX_SHEET_COLUMNS)))
    expect(strFromU8(parts['xl/worksheets/sheet1.xml']!)).toContain('<c r="XFD1" t="inlineStr">')
  })
})

describe('bytesToBase64', () => {
  it('writes bytes as base64 text', () => {
    expect(bytesToBase64(new Uint8Array([80, 75, 3, 4]))).toBe('UEsDBA==')
  })

  it('works on a run of bytes longer than one step', () => {
    const bytes = new Uint8Array(0x8000 + 10).fill(65)
    expect(atob(bytesToBase64(bytes))).toHaveLength(bytes.length)
  })
})
