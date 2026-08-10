import { describe, expect, it } from 'vitest'
import {
  exportFileName,
  toCsv,
  toCsvField,
  toJson,
  toScript,
  toTabSeparated,
  uniqueColumnNames,
} from '@/lib/export'
import type { ResultSet } from '@/types/api'

const result: ResultSet = {
  columns: [
    { name: 'id', typeName: 'int' },
    { name: 'name', typeName: 'text' },
  ],
  rows: [
    [1, 'Ada'],
    [2, null],
  ],
  truncated: false,
}

describe('toCsvField', () => {
  it('writes an empty field for a cell without a value', () => {
    expect(toCsvField(null)).toBe('')
  })

  it('leaves a plain value unquoted', () => {
    expect(toCsvField('abc')).toBe('abc')
    expect(toCsvField(7)).toBe('7')
  })

  it('quotes a field that holds a separator or a break', () => {
    expect(toCsvField('a,b')).toBe('"a,b"')
    expect(toCsvField('a\nb')).toBe('"a\nb"')
    expect(toCsvField('a\rb')).toBe('"a\rb"')
  })

  it('doubles a quote inside a field', () => {
    expect(toCsvField('say "hi"')).toBe('"say ""hi"""')
  })

  it('quotes a field with blank space at its edge', () => {
    expect(toCsvField(' a')).toBe('" a"')
  })
})

describe('toCsv', () => {
  it('writes a header and the rows', () => {
    expect(toCsv(result)).toBe('id,name\n1,Ada\n2,')
  })

  it('leaves the header out on request', () => {
    expect(toCsv(result, false)).toBe('1,Ada\n2,')
  })
})

describe('toJson', () => {
  it('writes one object for each row', () => {
    expect(JSON.parse(toJson(result))).toEqual([
      { id: 1, name: 'Ada' },
      { id: 2, name: null },
    ])
  })

  it('fills a missing cell with no value', () => {
    const short: ResultSet = { ...result, rows: [[1]] }
    expect(JSON.parse(toJson(short))).toEqual([{ id: 1, name: null }])
  })

  it('accepts another indent', () => {
    expect(toJson(result, 0)).not.toContain('\n  ')
  })
})

describe('toTabSeparated', () => {
  it('joins the cells with a tab and the rows with a break', () => {
    expect(toTabSeparated([['a', 'b'], [1, null]])).toBe('a\tb\n1\t')
  })
})

describe('uniqueColumnNames', () => {
  it('leaves names that differ alone', () => {
    expect(uniqueColumnNames(['a', 'b'])).toEqual(['a', 'b'])
  })

  it('numbers a name that repeats', () => {
    expect(uniqueColumnNames(['a', 'a', 'a'])).toEqual(['a', 'a_2', 'a_3'])
  })

  it('names a column that has no name', () => {
    expect(uniqueColumnNames(['', ''])).toEqual(['column', 'column_2'])
  })

  it('steps past a number that is already taken', () => {
    expect(uniqueColumnNames(['a', 'a_2', 'a'])).toEqual(['a', 'a_2', 'a_3'])
  })
})

describe('toScript', () => {
  it('ends every statement with a terminator', () => {
    expect(toScript(['SELECT 1', 'SELECT 2;'])).toBe('SELECT 1;\n\nSELECT 2;')
  })

  it('drops the empty statements', () => {
    expect(toScript(['', '  ', 'SELECT 1'])).toBe('SELECT 1;')
  })
})

describe('exportFileName', () => {
  const at = new Date(2026, 7, 10, 9, 5, 3)

  it('joins the name, the moment and the kind of file', () => {
    expect(exportFileName('Query 1', 'csv', at)).toBe('Query_1-20260810-090503.csv')
  })

  it('falls back on a name of its own', () => {
    expect(exportFileName('***', 'json', at)).toBe('result-20260810-090503.json')
  })

  it('uses the present moment when none is given', () => {
    expect(exportFileName('a', 'csv')).toMatch(/^a-\d{8}-\d{6}\.csv$/)
  })
})
