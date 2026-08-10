import { describe, expect, it } from 'vitest'
import {
  exportFileName,
  toCsv,
  toCsvField,
  toInsertStatements,
  toJson,
  toMarkdown,
  toScript,
  toSqlLiteral,
  toTabSeparated,
  uniqueColumnNames,
} from '@/lib/export'
import { Dialect, type ResultSet } from '@/types/api'

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
    expect(
      toTabSeparated([
        ['a', 'b'],
        [1, null],
      ]),
    ).toBe('a\tb\n1\t')
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

describe('toMarkdown', () => {
  it('writes a table with a header and a rule', () => {
    expect(toMarkdown(result)).toBe('| id | name |\n| --- | --- |\n| 1 | Ada |\n| 2 |  |')
  })

  it('escapes a bar and folds a line break', () => {
    const result = {
      columns: [{ name: 'text', typeName: 'text' }],
      rows: [['a|b'], ['one\ntwo']],
      truncated: false,
    }
    expect(toMarkdown(result)).toContain('| a\\|b |')
    expect(toMarkdown(result)).toContain('| one two |')
  })
})

describe('toSqlLiteral', () => {
  it('writes each kind of value', () => {
    expect(toSqlLiteral(null)).toBe('NULL')
    expect(toSqlLiteral(7)).toBe('7')
    expect(toSqlLiteral(Number.POSITIVE_INFINITY)).toBe('NULL')
    expect(toSqlLiteral(true)).toBe('1')
    expect(toSqlLiteral(false)).toBe('0')
    expect(toSqlLiteral("it's")).toBe("'it''s'")
  })
})

describe('toInsertStatements', () => {
  it('writes one statement for each row with the quotes of the dialect', () => {
    expect(toInsertStatements(result, 'dbo.people', Dialect.MsSql)).toBe(
      "INSERT INTO [dbo].[people] ([id], [name]) VALUES (1, 'Ada');\n" +
        'INSERT INTO [dbo].[people] ([id], [name]) VALUES (2, NULL);',
    )
  })

  it('quotes the name for the engine that uses back quotes', () => {
    expect(toInsertStatements(result, 'people', Dialect.MySql)).toContain(
      'INSERT INTO `people` (`id`, `name`)',
    )
  })

  it('fills a missing cell with NULL', () => {
    const result = {
      columns: [
        { name: 'a', typeName: 'int' },
        { name: 'b', typeName: 'int' },
      ],
      rows: [[1]],
      truncated: false,
    }
    expect(toInsertStatements(result, 't', Dialect.Sqlite)).toContain('VALUES (1, NULL)')
  })
})
