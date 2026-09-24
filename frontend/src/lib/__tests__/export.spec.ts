import { describe, expect, it } from 'vitest'
import {
  CSV_BOM,
  exportFileName,
  toCsv,
  toCsvField,
  toInsertStatements,
  toJson,
  toMarkdown,
  toScript,
  toSqlLiteral,
  toTabSeparated,
  startsAFormula,
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

  it('puts an apostrophe in front of a text that starts a formula', () => {
    expect(toCsvField('=SUM(A1:A9)')).toBe("'=SUM(A1:A9)")
    expect(toCsvField('+cmd')).toBe("'+cmd")
    expect(toCsvField('\rcmd')).toBe(`"'\rcmd"`)
    expect(toCsvField('-cmd')).toBe("'-cmd")
    expect(toCsvField('@name')).toBe("'@name")
    expect(toCsvField('\tpad')).toBe("'\tpad")
  })

  it('leaves a number as it is', () => {
    expect(toCsvField(-5)).toBe('-5')
    for (const number of ['-5', '+1', '-10.00', '-.5', '-5.', '-1e10', '+2.5E-3']) {
      expect(toCsvField(number)).toBe(number)
    }
    for (const text of ['-', '-.', '-1e', '-1.2.3', '-1x', '-e5', '-1-2']) {
      expect(toCsvField(text)).toBe(`'${text}`)
    }
    expect(toCsvField('a=b')).toBe('a=b')
  })
})

describe('startsAFormula', () => {
  it('answers false for an empty text', () => {
    expect(startsAFormula('')).toBe(false)
  })
})

describe('toCsv', () => {
  it('writes a header and the rows with the mark and the Excel line end', () => {
    expect(toCsv(result)).toBe(`${CSV_BOM}id,name\r\n1,Ada\r\n2,\r\n`)
  })

  it('leaves the header out on request', () => {
    expect(toCsv(result, false)).toBe(`${CSV_BOM}1,Ada\r\n2,\r\n`)
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
    ).toBe('a\tb\n1\tNULL')
  })

  it('keeps a cell without a value apart from an empty text', () => {
    expect(toTabSeparated([[null, '']])).toBe('NULL\t')
  })

  it('quotes a cell that holds a tab or a break, or that begins with a quote', () => {
    expect(toTabSeparated([['a\tb', 'one\ntwo', '"x" y', 'a "b"']])).toBe(
      '"a\tb"\t"one\ntwo"\t"""x"" y"\ta "b"',
    )
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
    expect(toSqlLiteral(null, Dialect.MsSql)).toBe('NULL')
    expect(toSqlLiteral(7, Dialect.MsSql)).toBe('7')
    expect(toSqlLiteral(Number.POSITIVE_INFINITY, Dialect.MsSql)).toBe('NULL')
    expect(toSqlLiteral(true, Dialect.MsSql)).toBe('1')
    expect(toSqlLiteral(false, Dialect.MySql)).toBe('0')
    expect(toSqlLiteral("it's", Dialect.Sqlite)).toBe("'it''s'")
    expect(toSqlLiteral({ a: 1 }, Dialect.Postgres)).toBe(`'{"a":1}'`)
  })

  it('writes a boolean as a word where the engine refuses a number', () => {
    expect(toSqlLiteral(true, Dialect.Postgres)).toBe('TRUE')
    expect(toSqlLiteral(false, Dialect.Postgres)).toBe('FALSE')
    expect(toSqlLiteral(true, Dialect.Athena)).toBe('TRUE')
  })

  it('writes an array in the form of each engine', () => {
    expect(toSqlLiteral([1, 2], Dialect.Postgres)).toBe("'{1,2}'")
    expect(toSqlLiteral([1, 2], Dialect.Athena)).toBe('ARRAY[1, 2]')
    expect(toSqlLiteral(['a', true], Dialect.Athena)).toBe("ARRAY['a', TRUE]")
    expect(toSqlLiteral([1, 2], Dialect.MySql)).toBe("'[1,2]'")
  })

  it('quotes each element of a PostgreSQL array that needs it', () => {
    const value = [
      ['a b', 'c,d'],
      ['', 'null'],
      ['say "hi"', 'back\\slash'],
      [null, Number.NaN],
      ["it's", false],
    ]
    expect(toSqlLiteral(value, Dialect.Postgres)).toBe(
      `'{{"a b","c,d"},{"","null"},{"say \\"hi\\"","back\\\\slash"},{NULL,NULL},{it''s,false}}'`,
    )
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

  it('writes the values in the form of the dialect', () => {
    const result = {
      columns: [
        { name: 'ok', typeName: 'bool' },
        { name: 'tags', typeName: '_int4' },
      ],
      rows: [[true, [1, 2]]],
      truncated: false,
    }
    expect(toInsertStatements(result, 't', Dialect.Postgres)).toBe(
      `INSERT INTO "t" ("ok", "tags") VALUES (TRUE, '{1,2}');`,
    )
  })
})
