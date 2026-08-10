import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub, connectionFixture, infoFixture } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const {
  columnNode,
  filterNodes,
  iconFor,
  isExpandable,
  tableNode,
  useExplorerStore,
  walk,
} = await import('@/stores/explorer')
type ExplorerNode = import('@/stores/explorer').ExplorerNode
const { useConnectionsStore } = await import('@/stores/connections')
const { useUiStore } = await import('@/stores/ui')
const { TableKind } = await import('@/types/api')

function node(overrides: Partial<ExplorerNode> = {}): ExplorerNode {
  return {
    key: 'k',
    label: 'label',
    kind: 'database',
    icon: 'mdi-database',
    children: [],
    loading: false,
    loaded: false,
    connectionId: 'c1',
    ...overrides,
  }
}

describe('iconFor', () => {
  it('gives an icon for every kind of node', () => {
    expect(iconFor('connection')).toBe('mdi-server')
    expect(iconFor('database')).toBe('mdi-database')
    expect(iconFor('schema')).toBe('mdi-folder-outline')
    expect(iconFor('table')).toBe('mdi-table')
    expect(iconFor('view')).toBe('mdi-table-eye')
    expect(iconFor('column')).toBe('mdi-table-column')
    expect(iconFor('column', true)).toBe('mdi-key-variant')
  })
})

describe('isExpandable', () => {
  it('holds for everything but a column', () => {
    expect(isExpandable(node({ kind: 'table' }))).toBe(true)
    expect(isExpandable(node({ kind: 'column' }))).toBe(false)
  })
})

describe('tableNode', () => {
  it('builds a node for a table and one for a view', () => {
    const table = tableNode({ name: 'orders', kind: TableKind.Table }, 'c1', 'Sales', 'dbo')
    expect(table.kind).toBe('table')
    expect(table.key).toBe('c1/Sales/dbo/orders')
    expect(table.children).toEqual([])
    expect(table.loaded).toBe(false)

    const view = tableNode({ name: 'big', kind: TableKind.View }, 'c1', 'Sales', undefined)
    expect(view.kind).toBe('view')
    expect(view.key).toBe('c1/Sales//big')
  })
})

describe('columnNode', () => {
  it('marks a key column and reports whether a column may hold no value', () => {
    const parent = node({ kind: 'table', key: 'c1/db/dbo/orders', table: 'orders' })
    const key = columnNode(
      { name: 'id', dataType: 'int', nullable: false, isPrimaryKey: true },
      parent,
    )
    expect(key.icon).toBe('mdi-key-variant')
    expect(key.hint).toBe('int not null')
    expect(key.key).toBe('c1/db/dbo/orders/id')
    expect(key.children).toBeUndefined()

    const plain = columnNode(
      { name: 'note', dataType: 'text', nullable: true, isPrimaryKey: false },
      parent,
    )
    expect(plain.icon).toBe('mdi-table-column')
    expect(plain.hint).toBe('text')
  })
})

describe('filterNodes', () => {
  const tree = [
    node({
      key: 'root',
      label: 'Server',
      kind: 'connection',
      children: [
        node({ key: 'db', label: 'Sales', children: [node({ key: 't', label: 'orders', kind: 'table' })] }),
        node({ key: 'db2', label: 'Other', children: [] }),
      ],
    }),
  ]

  it('keeps the whole tree for an empty filter', () => {
    expect(filterNodes(tree, '   ')).toBe(tree)
  })

  it('keeps the path down to a match', () => {
    const filtered = filterNodes(tree, 'orders')
    expect(filtered).toHaveLength(1)
    expect(filtered[0]?.children).toHaveLength(1)
    expect(filtered[0]?.children?.[0]?.children?.[0]?.label).toBe('orders')
  })

  it('drops a branch that holds no match', () => {
    const filtered = filterNodes(tree, 'sales')
    expect(filtered[0]?.children?.map((child) => child.label)).toEqual(['Sales'])
  })

  it('gives an empty list when nothing matches', () => {
    expect(filterNodes(tree, 'nothing')).toEqual([])
  })

  it('keeps a leaf without children as a leaf', () => {
    const leaves = [node({ key: 'c', label: 'id', kind: 'column', children: undefined })]
    expect(filterNodes(leaves, 'id')[0]?.children).toBeUndefined()
  })
})

describe('walk', () => {
  it('visits every node in the tree', () => {
    const seen: string[] = []
    walk(
      [node({ key: 'a', children: [node({ key: 'b', children: undefined })] })],
      (visited) => seen.push(visited.key),
    )
    expect(seen).toEqual(['a', 'b'])
  })
})

describe('explorer store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
  })

  async function readyStore(supportsSchemas = true) {
    apiStub.listActiveConnections.mockResolvedValue([infoFixture('c1', supportsSchemas)])
    const connections = useConnectionsStore()
    await connections.load()
    return useExplorerStore()
  }

  it('adds one root for each open connection and adds no duplicate', async () => {
    const explorer = await readyStore()
    const first = explorer.addRoot('c1')
    expect(first.label).toBe('Server')
    expect(explorer.addRoot('c1').key).toBe(first.key)
    expect(explorer.roots).toHaveLength(1)
  })

  it('names a root by its identifier when the record is gone', async () => {
    const explorer = await readyStore()
    expect(explorer.addRoot('unknown').label).toBe('unknown')
  })

  it('removes a root and empties the tree', async () => {
    const explorer = await readyStore()
    explorer.addRoot('c1')
    explorer.removeRoot('c1')
    expect(explorer.roots).toEqual([])

    explorer.addRoot('c1')
    explorer.filter = 'x'
    explorer.clear()
    expect(explorer.roots).toEqual([])
    expect(explorer.filter).toBe('')
  })

  it('reads the databases below a connection', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'Other' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    expect(root.children?.map((child) => child.label)).toEqual(['Sales', 'Other'])
    expect(root.loaded).toBe(true)
    expect(root.loading).toBe(false)
    expect(explorer.loading).toBe(false)
  })

  it('reads the schemas below a database when the engine has them', async () => {
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    const explorer = await readyStore(true)
    const database = node({ kind: 'database', database: 'Sales', label: 'Sales' })
    await explorer.expand(database)
    expect(apiStub.listSchemas).toHaveBeenCalledWith('c1', 'Sales')
    expect(database.children?.[0]?.kind).toBe('schema')
  })

  it('reads the tables below a database when the engine has no schemas', async () => {
    apiStub.listTables.mockResolvedValue([{ name: 'orders', kind: TableKind.Table }])
    const explorer = await readyStore(false)
    const database = node({ kind: 'database', database: 'shop', label: 'shop' })
    await explorer.expand(database)
    expect(apiStub.listTables).toHaveBeenCalledWith('c1', 'shop', null)
    expect(database.children?.[0]?.kind).toBe('table')
  })

  it('takes the name of a database from its label when the field is absent', async () => {
    apiStub.listTables.mockResolvedValue([])
    const explorer = await readyStore(false)
    await explorer.expand(node({ kind: 'database', label: 'shop', database: undefined }))
    expect(apiStub.listTables).toHaveBeenCalledWith('c1', 'shop', null)
  })

  it('names the database when it reads the tables of a schema', async () => {
    apiStub.listTables.mockResolvedValue([{ name: 'orders', kind: TableKind.View }])
    const explorer = await readyStore()
    const schema = node({ kind: 'schema', database: 'Sales', schema: 'dbo', label: 'dbo' })
    await explorer.expand(schema)
    expect(apiStub.listTables).toHaveBeenCalledWith('c1', 'Sales', 'dbo')
    expect(schema.children?.[0]?.kind).toBe('view')
  })

  it('uses an empty database name when a schema carries none', async () => {
    apiStub.listTables.mockResolvedValue([])
    const explorer = await readyStore()
    await explorer.expand(node({ kind: 'schema', schema: 'dbo', database: undefined }))
    expect(apiStub.listTables).toHaveBeenCalledWith('c1', '', 'dbo')
  })

  it('reads the columns below a table', async () => {
    apiStub.listColumns.mockResolvedValue([
      { name: 'id', dataType: 'int', nullable: false, isPrimaryKey: true },
    ])
    const explorer = await readyStore()
    const table = node({
      kind: 'table',
      database: 'Sales',
      schema: 'dbo',
      table: 'orders',
      label: 'orders',
    })
    await explorer.expand(table)
    expect(apiStub.listColumns).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(table.children?.[0]?.kind).toBe('column')
  })

  it('takes the name of a table from its label when the field is absent', async () => {
    apiStub.listColumns.mockResolvedValue([])
    const explorer = await readyStore()
    await explorer.expand(node({ kind: 'table', label: 'orders', table: undefined }))
    expect(apiStub.listColumns).toHaveBeenCalledWith('c1', '', null, 'orders')
  })

  it('reads a branch once and no more', async () => {
    apiStub.listDatabases.mockResolvedValue([])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    await explorer.expand(root)
    expect(apiStub.listDatabases).toHaveBeenCalledTimes(1)
  })

  it('does not read a branch that is already reading', async () => {
    const explorer = await readyStore()
    const busy = node({ loading: true })
    await explorer.expand(busy)
    expect(apiStub.listDatabases).not.toHaveBeenCalled()
  })

  it('does not read a column, which holds nothing below it', async () => {
    const explorer = await readyStore()
    await explorer.expand(node({ kind: 'column' }))
    expect(apiStub.listColumns).not.toHaveBeenCalled()
  })

  it('reports a failure and leaves the branch closed', async () => {
    apiStub.listDatabases.mockRejectedValue({
      kind: 'notConnected',
      message: 'gone',
      detail: null,
    })
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    expect(root.loaded).toBe(false)
    expect(root.children).toEqual([])
    expect(useUiStore().notices[0]?.level).toBe('error')
  })

  it('reads a branch again on request', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'New' }])
    await explorer.refresh(root)
    expect(root.children).toHaveLength(2)
  })

  it('treats a connection the store does not know as one without schemas', async () => {
    apiStub.listTables.mockResolvedValue([])
    const explorer = useExplorerStore()
    await explorer.expand(node({ kind: 'database', database: 'shop', connectionId: 'other' }))
    expect(apiStub.listTables).toHaveBeenCalledWith('other', 'shop', null)
  })

  it('builds the names the editor offers, without repeating one', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }])
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    apiStub.listTables.mockResolvedValue([{ name: 'orders', kind: TableKind.Table }])
    apiStub.listColumns.mockResolvedValue([
      { name: 'id', dataType: 'int', nullable: false, isPrimaryKey: true },
    ])

    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    const database = root.children![0]!
    await explorer.expand(database)
    const schema = database.children![0]!
    await explorer.expand(schema)
    const table = schema.children![0]!
    await explorer.expand(table)

    expect(explorer.schemaIndex).toEqual({
      databases: ['Sales'],
      schemas: ['dbo'],
      tables: [{ name: 'orders', qualifier: 'Sales.dbo' }],
      columns: [{ name: 'id', table: 'orders', dataType: 'int not null' }],
    })

    // A second root over the same names adds nothing new.
    explorer.roots = [...explorer.roots, ...explorer.roots]
    expect(explorer.schemaIndex.databases).toEqual(['Sales'])
  })

  it('reports a column without a type as one without a hint', async () => {
    const explorer = useExplorerStore()
    explorer.roots = [node({ kind: 'column', label: 'id', hint: undefined, table: undefined })]
    expect(explorer.schemaIndex.columns).toEqual([{ name: 'id', table: '', dataType: '' }])
  })

  it('keeps the reading flag while another branch still reads', async () => {
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    root.children = [node({ key: 'busy', loading: true, children: undefined })]
    apiStub.listDatabases.mockResolvedValue([])
    await explorer.expand(root)
    expect(explorer.loading).toBe(true)
  })

  it('shows only the nodes that match the filter', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'Other' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    explorer.filter = 'sales'
    expect(explorer.visibleNodes[0]?.children).toHaveLength(1)
  })
})
