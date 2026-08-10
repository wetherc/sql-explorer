import { describe, expect, it } from 'vitest'
import {
  SQL_KEYWORDS,
  completionsFor,
  emptySchemaIndex,
  formatSql,
  formatterDialect,
  isPlainIdentifier,
  quoteIdentifier,
  qualifierBefore,
  quoteIfNeeded,
  statementAt,
  statementBounds,
  tableAliases,
  wordBefore,
  type SchemaIndex,
} from '@/lib/sql'
import { Dialect } from '@/types/api'

describe('formatterDialect', () => {
  it('names the dialect of the formatter for each engine', () => {
    expect(formatterDialect(Dialect.MsSql)).toBe('transactsql')
    expect(formatterDialect(Dialect.MySql)).toBe('mysql')
    expect(formatterDialect(Dialect.Postgres)).toBe('postgresql')
    expect(formatterDialect(Dialect.Sqlite)).toBe('sqlite')
    expect(formatterDialect(Dialect.Athena)).toBe('trino')
  })
})

describe('formatSql', () => {
  it('lays out a statement and puts the keywords in capitals', () => {
    expect(formatSql('select a from t where b=1', Dialect.Postgres)).toBe(
      'SELECT\n  a\nFROM\n  t\nWHERE\n  b = 1',
    )
  })

  it('keeps the dialect of the connection', () => {
    expect(formatSql('select top 1 [a] from [t]', Dialect.MsSql)).toContain('SELECT\n  TOP 1 [a]')
  })

  it('throws when the text cannot be read', () => {
    expect(() => formatSql('SELECT * FROM (', Dialect.Sqlite)).toThrow(/Parse error/)
  })
})

describe('quoteIdentifier', () => {
  it('uses the quotes of each engine', () => {
    expect(quoteIdentifier('dbo', Dialect.MsSql)).toBe('[dbo]')
    expect(quoteIdentifier('db', Dialect.MySql)).toBe('`db`')
    expect(quoteIdentifier('pub', Dialect.Postgres)).toBe('"pub"')
    expect(quoteIdentifier('t', Dialect.Sqlite)).toBe('"t"')
    expect(quoteIdentifier('t', Dialect.Athena)).toBe('"t"')
  })

  it('doubles a quote inside a name', () => {
    expect(quoteIdentifier('a]b', Dialect.MsSql)).toBe('[a]]b]')
    expect(quoteIdentifier('a`b', Dialect.MySql)).toBe('`a``b`')
    expect(quoteIdentifier('a"b', Dialect.Postgres)).toBe('"a""b"')
  })
})

describe('isPlainIdentifier', () => {
  it('accepts a name that needs no quotes', () => {
    expect(isPlainIdentifier('orders')).toBe(true)
    expect(isPlainIdentifier('_a1')).toBe(true)
  })

  it('refuses a name that needs quotes', () => {
    expect(isPlainIdentifier('1a')).toBe(false)
    expect(isPlainIdentifier('a b')).toBe(false)
    expect(isPlainIdentifier('')).toBe(false)
  })
})

describe('quoteIfNeeded', () => {
  it('quotes only the names that need it', () => {
    expect(quoteIfNeeded('orders', Dialect.MsSql)).toBe('orders')
    expect(quoteIfNeeded('order items', Dialect.MsSql)).toBe('[order items]')
  })
})

describe('statementBounds', () => {
  it('splits on a terminator', () => {
    expect(statementBounds('SELECT 1; SELECT 2')).toEqual([
      [0, 8],
      [9, 18],
    ])
  })

  it('gives one block for an empty script', () => {
    expect(statementBounds('')).toEqual([[0, 0]])
  })

  it('gives one block when the script ends on a terminator', () => {
    expect(statementBounds('SELECT 1;')).toEqual([[0, 8]])
  })

  it('keeps a terminator inside a text', () => {
    expect(statementBounds("SELECT 'a;b'")).toEqual([[0, 12]])
    expect(statementBounds('SELECT "a;b"')).toEqual([[0, 12]])
    expect(statementBounds('SELECT `a;b`')).toEqual([[0, 12]])
  })

  it('keeps a doubled quote inside a text', () => {
    expect(statementBounds("SELECT 'it''s; ok'")).toEqual([[0, 18]])
  })

  it('keeps a terminator inside a comment', () => {
    expect(statementBounds('SELECT 1 -- a; b\n')).toEqual([[0, 17]])
    expect(statementBounds('SELECT /* a; b */ 1')).toEqual([[0, 19]])
  })

  it('reads a comment that never closes', () => {
    expect(statementBounds('SELECT /* a; b')).toEqual([[0, 14]])
    expect(statementBounds('SELECT -- a; b')).toEqual([[0, 14]])
  })
})

describe('statementAt', () => {
  const script = 'SELECT 1;\nSELECT 2;\nSELECT 3'

  it('finds the statement that holds the position', () => {
    expect(statementAt(script, 0)).toBe('SELECT 1')
    expect(statementAt(script, 12)).toBe('SELECT 2')
    expect(statementAt(script, 25)).toBe('SELECT 3')
  })

  it('keeps the position inside the script', () => {
    expect(statementAt(script, -5)).toBe('SELECT 1')
    expect(statementAt(script, 5000)).toBe('SELECT 3')
  })

  it('gives the whole script when no block holds the position', () => {
    expect(statementAt('SELECT 1;', 9)).toBe('SELECT 1;')
  })
})

describe('wordBefore', () => {
  it('reads the word that ends at the position', () => {
    expect(wordBefore('SELECT ord', 10)).toBe('ord')
    expect(wordBefore('SELECT ', 7)).toBe('')
  })

  it('keeps the position inside the text', () => {
    expect(wordBefore('abc', -1)).toBe('')
    expect(wordBefore('abc', 99)).toBe('abc')
  })
})

describe('completionsFor', () => {
  const index: SchemaIndex = {
    databases: ['Sales'],
    schemas: ['sales_reports'],
    tables: [{ name: 'salesOrder', qualifier: 'Sales.dbo' }],
    columns: [{ name: 'sale_total', table: 'salesOrder', qualifier: '', dataType: 'money' }],
  }

  it('offers the objects before the keywords', () => {
    const items = completionsFor('sal', index, Dialect.MsSql)
    expect(items.map((item) => item.kind)).toEqual(['column', 'table', 'schema', 'database'])
  })

  it('offers everything for an empty prefix', () => {
    const items = completionsFor('', index, Dialect.MsSql)
    expect(items).toHaveLength(4 + SQL_KEYWORDS.length)
  })

  it('offers the keywords that match', () => {
    const items = completionsFor('sel', index, Dialect.MsSql)
    expect(items).toEqual([
      { label: 'SELECT', detail: 'keyword', insertText: 'SELECT', kind: 'keyword' },
    ])
  })

  it('quotes a name that needs quotes', () => {
    const awkward: SchemaIndex = {
      ...emptySchemaIndex(),
      tables: [{ name: 'order items', qualifier: 'dbo' }],
    }
    expect(completionsFor('order', awkward, Dialect.MsSql)[0]?.insertText).toBe('[order items]')
  })

  it('describes a column with its type and its table', () => {
    expect(completionsFor('sale_', index, Dialect.MsSql)[0]?.detail).toBe('money in salesOrder')
  })
})

describe('emptySchemaIndex', () => {
  it('starts with nothing in it', () => {
    expect(emptySchemaIndex()).toEqual({
      databases: [],
      schemas: [],
      tables: [],
      columns: [],
    })
  })
})

describe('completionsFor with a dialect that quotes differently', () => {
  it('quotes an awkward name for each engine', () => {
    const index: SchemaIndex = {
      ...emptySchemaIndex(),
      databases: ['my db'],
      schemas: ['my schema'],
      columns: [{ name: 'my column', table: 't', qualifier: '', dataType: 'text' }],
    }
    expect(completionsFor('my c', index, Dialect.MySql)[0]?.insertText).toBe('`my column`')
    expect(completionsFor('my s', index, Dialect.Postgres)[0]?.insertText).toBe('"my schema"')
    expect(completionsFor('my d', index, Dialect.Sqlite)[0]?.insertText).toBe('"my db"')
  })
})

describe('qualifierBefore', () => {
  it('reads the name in front of the full stop at the cursor', () => {
    expect(qualifierBefore('SELECT o.', 9)).toBe('o')
    expect(qualifierBefore('SELECT o.tot', 12)).toBe('o')
    expect(qualifierBefore('SELECT [Sales].[dbo].', 21)).toBe('dbo')
    expect(qualifierBefore('SELECT "public".', 16)).toBe('public')
    expect(qualifierBefore('SELECT `shop`.', 14)).toBe('shop')
  })

  it('gives nothing when no full stop stands before the cursor', () => {
    expect(qualifierBefore('SELECT tot', 10)).toBe('')
    expect(qualifierBefore('', 0)).toBe('')
    expect(qualifierBefore('SELECT o.tot', 99)).toBe('o')
  })

  it('gives nothing when the quoted name never opened', () => {
    expect(qualifierBefore('SELECT dbo].', 12)).toBe('')
  })
})

describe('tableAliases', () => {
  it('reads the alias of a relation, with and without the word AS', () => {
    const aliases = tableAliases(
      'SELECT * FROM Sales.dbo.Orders AS o JOIN Customers c ON c.id = o.customer',
      Dialect.MsSql,
    )
    expect(aliases.get('o')).toBe('Orders')
    expect(aliases.get('c')).toBe('Customers')
    // The name of a relation answers for itself as well.
    expect(aliases.get('orders')).toBe('Orders')
    expect(aliases.get('customers')).toBe('Customers')
  })

  it('reads a quoted name and the brackets of MS SQL Server', () => {
    const aliases = tableAliases('SELECT * FROM [Sales].[dbo].[Order Lines] AS l', Dialect.MsSql)
    expect(aliases.get('l')).toBe('Order Lines')
    expect(tableAliases('SELECT * FROM "public"."orders" o', Dialect.Postgres).get('o')).toBe(
      'orders',
    )
  })

  it('steps over the comments and the literals', () => {
    const aliases = tableAliases(
      "-- FROM nothing\n/* FROM nothing */ SELECT 'FROM x' FROM orders o",
      Dialect.Postgres,
    )
    expect([...aliases.keys()]).toEqual(['orders', 'o'])
  })

  it('leaves out a clause that names no relation', () => {
    expect(tableAliases('SELECT * FROM', Dialect.Postgres).size).toBe(0)
    expect(tableAliases('SELECT * FROM (SELECT 1) AS x', Dialect.Postgres).size).toBe(0)
  })

  it('stops the name of a relation at the next word of the clause', () => {
    const aliases = tableAliases('SELECT * FROM orders WHERE id = 1', Dialect.Postgres)
    expect([...aliases.keys()]).toEqual(['orders'])
    const joined = tableAliases('SELECT * FROM a JOIN b ON a.id = b.id', Dialect.Postgres)
    expect([...joined.keys()]).toEqual(['a', 'b'])
  })

  it('reads a relation that stands at the end of the statement', () => {
    const aliases = tableAliases('SELECT * FROM orders', Dialect.Postgres)
    expect(aliases.get('orders')).toBe('orders')
  })

  it('steps over a quoted name that stands outside a clause', () => {
    const aliases = tableAliases('SELECT "id" FROM orders o', Dialect.Postgres)
    expect(aliases.get('o')).toBe('orders')
  })

  it('reads a name of two parts as the last part', () => {
    const aliases = tableAliases('SELECT * FROM shop.orders', Dialect.MySql)
    expect(aliases.get('orders')).toBe('orders')
  })
})

describe('completionsFor with a qualifier', () => {
  const index = {
    databases: ['Sales'],
    schemas: ['dbo', 'staging'],
    tables: [
      { name: 'Orders', qualifier: 'Sales.dbo' },
      { name: 'Orders', qualifier: 'Sales.staging' },
    ],
    columns: [
      { name: 'total', table: 'Orders', qualifier: 'Sales.dbo', dataType: 'money' },
      { name: 'raw', table: 'Orders', qualifier: 'Sales.staging', dataType: 'text' },
      { name: 'note', table: 'Customers', qualifier: 'Sales.dbo', dataType: 'text' },
    ],
  }

  it('gives the columns of the relation an alias stands for', () => {
    const aliases = new Map([['o', 'Orders']])
    const items = completionsFor('', index, Dialect.MsSql, { qualifier: 'o', aliases })
    expect(items.map((item) => item.label)).toEqual(['total', 'raw'])
  })

  it('gives the columns of a relation the qualifier names itself', () => {
    const items = completionsFor('', index, Dialect.MsSql, { qualifier: 'Customers' })
    expect(items.map((item) => item.label)).toEqual(['note'])
  })

  it('keeps the prefix while a qualifier stands', () => {
    const items = completionsFor('no', index, Dialect.MsSql, { qualifier: 'Customers' })
    expect(items.map((item) => item.label)).toEqual(['note'])
    expect(completionsFor('zz', index, Dialect.MsSql, { qualifier: 'Customers' })).toEqual([])
  })

  it('gives the relations of a schema when the qualifier names one', () => {
    const items = completionsFor('', { ...index, columns: [] }, Dialect.MsSql, {
      qualifier: 'staging',
    })
    expect(items.map((item) => item.detail)).toEqual(['Sales.staging'])
  })

  it('puts the columns of the relations of the statement in front', () => {
    const aliases = new Map([['customers', 'Customers']])
    const items = completionsFor('', index, Dialect.MsSql, { aliases })
    expect(items[0]?.label).toBe('note')
  })

  it('offers every name when the statement holds no relation', () => {
    const items = completionsFor('', index, Dialect.MsSql, { qualifier: '  ' })
    expect(items.map((item) => item.label).slice(0, 3)).toEqual(['total', 'raw', 'note'])
  })
})
