import { describe, expect, it } from 'vitest'
import { unzipSync, strFromU8 } from 'fflate'
import {
  bytesToBase64,
  columnName,
  escapeXml,
  sheetName,
  sheetXml,
  stripForbiddenXml,
  toXlsx,
  workbookXml,
} from '@/lib/xlsx'
import type { ResultSet } from '@/types/api'

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
