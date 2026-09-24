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
  statementAround,
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

  it('reads a text and a name that never close', () => {
    expect(statementBounds("SELECT 'a; b")).toEqual([[0, 12]])
    expect(statementBounds('SELECT [a; b', Dialect.MsSql)).toEqual([[0, 12]])
  })

  it('reads a comment that never closes', () => {
    expect(statementBounds('SELECT /* a; b')).toEqual([[0, 14]])
    expect(statementBounds('SELECT -- a; b')).toEqual([[0, 14]])
  })

  it('ends a statement on the batch separator of MS SQL Server', () => {
    const script = 'SELECT 1\nGO\nSELECT 2'
    expect(statementBounds(script, Dialect.MsSql)).toEqual([
      [0, 9],
      [12, 20],
    ])
    // The separator carries a count and a comment.
    expect(statementBounds('SELECT 1\nGO 2 -- twice\nSELECT 2', Dialect.MsSql)).toEqual([
      [0, 9],
      [23, 31],
    ])
    // Another dialect holds no separator.
    expect(statementBounds(script, Dialect.Postgres)).toEqual([[0, 20]])
    expect(statementBounds(script)).toEqual([[0, 20]])
  })

  it('keeps a line that holds more than the batch separator', () => {
    for (const script of ['SELECT 1\nGOTO done', 'SELECT 1\nGO SELECT 2', 'SELECT 1 GO']) {
      expect(statementBounds(script, Dialect.MsSql)).toEqual([[0, script.length]])
    }
  })

  it('keeps a batch separator that stands inside a text', () => {
    expect(statementBounds("SELECT 'a\nGO\nb'", Dialect.MsSql)).toEqual([[0, 15]])
    expect(statementBounds('SELECT 1 -- GO\n', Dialect.MsSql)).toEqual([[0, 15]])
  })

  it('keeps a terminator inside a name in brackets on MS SQL Server', () => {
    const script = 'SELECT [a;b] FROM t'
    expect(statementBounds(script, Dialect.MsSql)).toEqual([[0, script.length]])
    // A doubled closing bracket stays inside the name.
    const doubled = 'SELECT [a]];b] FROM t'
    expect(statementBounds(doubled, Dialect.MsSql)).toEqual([[0, doubled.length]])
    // Another dialect reads no name in brackets.
    expect(statementBounds(script, Dialect.Postgres)).toEqual([
      [0, 9],
      [10, 19],
    ])
  })

  it('keeps a terminator inside a body that a dollar tag encloses', () => {
    const script = 'CREATE FUNCTION f() RETURNS int AS $body$ BEGIN RETURN 1; END $body$;'
    expect(statementBounds(script, Dialect.Postgres)).toEqual([[0, 68]])
    // A tag that never closes runs to the end of the script.
    expect(statementBounds('SELECT $$a; b', Dialect.Postgres)).toEqual([[0, 13]])
    // A dollar sign that opens no tag holds no text.
    expect(statementBounds('SELECT $1; SELECT $2', Dialect.Postgres)).toEqual([
      [0, 9],
      [10, 20],
    ])
  })

  it('reads a block comment inside a block comment on PostgreSQL', () => {
    const script = 'SELECT /* a /* b */ ; */ 1'
    expect(statementBounds(script, Dialect.Postgres)).toEqual([[0, script.length]])
    expect(statementBounds(script, Dialect.MsSql)).toEqual([[0, script.length]])
    // A dialect without nested comments ends the comment at the first mark.
    expect(statementBounds(script, Dialect.Sqlite)).toEqual([
      [0, 20],
      [21, 26],
    ])
  })

  it('reads the escapes and the comments of MySQL', () => {
    // A backslash holds the quote that follows it inside the text.
    const escaped = "SELECT 'a\\'; b'"
    expect(statementBounds(escaped, Dialect.MySql)).toEqual([[0, escaped.length]])
    // A number sign starts a comment.
    const hash = 'SELECT 1 # a; b'
    expect(statementBounds(hash, Dialect.MySql)).toEqual([[0, hash.length]])
    expect(statementBounds(hash, Dialect.Postgres)).toEqual([
      [0, 12],
      [13, 15],
    ])
  })

  it('follows the DELIMITER command of MySQL', () => {
    const script =
      'DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;\nSELECT 2;\n'
    expect(statementBounds(script, Dialect.MySql)).toEqual([
      [13, 53],
      [68, 76],
      [77, 78],
    ])
    expect(script.slice(13, 53)).toBe('CREATE PROCEDURE p() BEGIN SELECT 1; END')
    // Another dialect holds the command as text and splits on the semicolon.
    expect(statementBounds('DELIMITER $$\nSELECT 1;', Dialect.Sqlite)).toEqual([[0, 21]])
  })

  it('reads a DELIMITER command that follows a comment', () => {
    const script =
      '-- make p\nDELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;'
    const bounds = statementBounds(script, Dialect.MySql)
    expect(bounds.map(([from, to]) => script.slice(from, to))).toEqual([
      'CREATE PROCEDURE p() BEGIN SELECT 1; END',
    ])
    const block = '/* two */\nDELIMITER //\nSELECT 1; SELECT 2//'
    expect(
      statementBounds(block, Dialect.MySql).map(([from, to]) => block.slice(from, to)),
    ).toEqual(['SELECT 1; SELECT 2'])
    // MySQL runs the text of an executable comment, so the word after it
    // belongs to that statement.
    const executable = '/*!40101 SET x = 1 */\nDELIMITER //\nSELECT 1;'
    expect(statementBounds(executable, Dialect.MySql)).toEqual([[0, executable.length - 1]])
    // Text in front of the word keeps it as text.
    const text = 'SELECT 1\nDELIMITER //\nSELECT 2;'
    expect(statementBounds(text, Dialect.MySql)).toEqual([[0, text.length - 1]])
  })

  it('holds a line that carries no terminator for the DELIMITER command', () => {
    // The word alone, the word with a longer word behind it, and the word
    // without a terminator all stay text.
    for (const script of ['DELIMITER \nSELECT 1;', 'DELIMITERS $$\nSELECT 1;']) {
      expect(statementBounds(script, Dialect.MySql)).toEqual([[0, script.length - 1]])
    }
    // The command can end the script.
    expect(statementBounds('SELECT 1;\nDELIMITER $$', Dialect.MySql)).toEqual([[0, 8]])
  })

  it('reads a backtick as a quote on MySQL alone', () => {
    expect(statementBounds('SELECT `a;b`', Dialect.MySql)).toEqual([[0, 12]])
    expect(statementBounds('SELECT `a;b`', Dialect.Postgres)).toEqual([
      [0, 9],
      [10, 12],
    ])
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

  it('gives the last statement when the position follows the last semicolon', () => {
    expect(statementAt('SELECT 1;', 9)).toBe('SELECT 1')
    expect(statementAt('SELECT 1;\nSELECT 2;\n', 20)).toBe('SELECT 2')
    expect(statementAt('SELECT 1;\n   ', 13)).toBe('SELECT 1')
  })

  it('gives the first statement when nothing stands in front of the position', () => {
    expect(statementAt(' ;SELECT 1', 0)).toBe('SELECT 1')
  })

  it('gives an empty text when the script holds no statement', () => {
    expect(statementAt('', 0)).toBe('')
    expect(statementAt('  ;  ', 3)).toBe('')
    expect(statementAt('GO\n', 0, Dialect.MsSql)).toBe('')
  })

  it('never sends the batch separator with the statement', () => {
    const script = 'SELECT 1\nGO\nSELECT 2'
    expect(statementAt(script, 0, Dialect.MsSql)).toBe('SELECT 1')
    // A cursor on the line of the separator gives the statement in front.
    expect(statementAt(script, 10, Dialect.MsSql)).toBe('SELECT 1')
    expect(statementAt(script, 15, Dialect.MsSql)).toBe('SELECT 2')
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

  it('stops at the number of names the caller asks for', () => {
    // The limit stops the walk of each list, the keywords among them.
    expect(completionsFor('', index, Dialect.MsSql, { limit: 2 })).toHaveLength(2)
    expect(completionsFor('sal', index, Dialect.MsSql, { limit: 1 })).toHaveLength(1)
    expect(completionsFor('sales_', index, Dialect.MsSql, { limit: 1 })).toHaveLength(1)
    expect(completionsFor('Sale', index, Dialect.MsSql, { limit: 3 })).toHaveLength(3)

    // A qualifier names a relation, and the limit holds its columns too.
    const wide: SchemaIndex = {
      ...emptySchemaIndex(),
      tables: [
        { name: 'orders', qualifier: 'Sales.dbo' },
        { name: 'items', qualifier: 'Sales.dbo' },
      ],
      columns: [
        { name: 'one', table: 'orders', qualifier: 'Sales.dbo', dataType: 'int' },
        { name: 'two', table: 'orders', qualifier: 'Sales.dbo', dataType: 'int' },
      ],
    }
    expect(completionsFor('', wide, Dialect.MsSql, { qualifier: 'orders', limit: 1 })).toHaveLength(
      1,
    )
    // The qualifier names a schema whose columns are not held, so the
    // relations of it are offered and the limit holds them too.
    const schemaOnly: SchemaIndex = {
      ...emptySchemaIndex(),
      tables: [
        { name: 'orders', qualifier: 'Sales.dbo' },
        { name: 'items', qualifier: 'Sales.dbo' },
      ],
    }
    expect(
      completionsFor('', schemaOnly, Dialect.MsSql, { qualifier: 'dbo', limit: 1 }),
    ).toHaveLength(1)
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

describe('statementAround', () => {
  it('gives the statement that holds a place, with the place inside it', () => {
    const script = 'SELECT 1;\nSELECT 2 FROM t'

    const first = statementAround(script, 3)
    expect(first.text).toBe('SELECT 1')
    expect(first.offset).toBe(3)

    const second = statementAround(script, script.length)
    expect(second.text).toBe('\nSELECT 2 FROM t')
    expect(second.offset).toBe(second.text.length)
  })

  it('gives the whole text of a script that holds one statement', () => {
    expect(statementAround('SELECT 1', 100)).toEqual({ text: 'SELECT 1', offset: 8 })
    expect(statementAround('', 0)).toEqual({ text: '', offset: 0 })
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

  it('reads every relation of a list with commas', () => {
    const aliases = tableAliases('SELECT * FROM a, b AS second, c third', Dialect.Postgres)
    expect([...aliases.keys()]).toEqual(['a', 'b', 'second', 'c', 'third'])
    expect(aliases.get('second')).toBe('b')
    expect(aliases.get('third')).toBe('c')
  })

  it('stops a list with commas at a name it cannot read', () => {
    const aliases = tableAliases('SELECT * FROM a, (SELECT 1) AS x', Dialect.Postgres)
    expect([...aliases.keys()]).toEqual(['a'])
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
