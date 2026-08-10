import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub, connectionFixture, infoFixture } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const {
  columnNode,
  constraintHint,
  filterNodes,
  folderNode,
  iconFor,
  isExpandable,
  leafNode,
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
    expect(iconFor('folder')).toBe('mdi-folder-outline')
    expect(iconFor('routine')).toBe('mdi-function-variant')
    expect(iconFor('index')).toBe('mdi-sort-alphabetical-variant')
    expect(iconFor('constraint')).toBe('mdi-key-chain')
    expect(iconFor('partition')).toBe('mdi-file-tree-outline')
  })
})

describe('isExpandable', () => {
  it('holds for a node that can hold children', () => {
    expect(isExpandable(node({ kind: 'table' }))).toBe(true)
    expect(isExpandable(node({ kind: 'folder' }))).toBe(true)
    for (const kind of ['column', 'routine', 'index', 'constraint', 'partition'] as const) {
      expect(isExpandable(node({ kind }))).toBe(false)
    }
  })
})

describe('folderNode and leafNode', () => {
  it('carries the place of the parent down to the child', () => {
    const table = node({
      kind: 'table',
      key: 'c1/Sales/dbo/table/orders',
      database: 'Sales',
      schema: 'dbo',
      table: 'orders',
    })
    const folder = folderNode('Indexes', 'indexes', table)
    expect(folder.key).toBe('c1/Sales/dbo/table/orders/indexes')
    expect(folder.folder).toBe('indexes')
    expect(folder.table).toBe('orders')

    const leaf = leafNode('by_total', 'index', folder, 'total')
    expect(leaf.key).toBe('c1/Sales/dbo/table/orders/indexes/by_total')
    expect(leaf.children).toBeUndefined()
    expect(leaf.hint).toBe('total')
    expect(leaf.schema).toBe('dbo')
  })
})

describe('constraintHint', () => {
  it('names the kind, the columns and the detail', () => {
    expect(constraintHint({ name: 'pk', kind: 'primaryKey', columns: ['id'], detail: null })).toBe(
      'primary key \u00b7 id',
    )
    expect(
      constraintHint({
        name: 'fk',
        kind: 'foreignKey',
        columns: ['customer'],
        detail: 'customers(id)',
      }),
    ).toBe('foreign key \u00b7 customer \u00b7 customers(id)')
    expect(constraintHint({ name: 'u', kind: 'unique', columns: [], detail: null })).toBe('unique')
    expect(constraintHint({ name: 'c', kind: 'check', columns: [], detail: 'total > 0' })).toBe(
      'check \u00b7 total > 0',
    )
  })
})

describe('tableNode', () => {
  it('builds a node for a table and one for a view', () => {
    const table = tableNode({ name: 'orders', kind: TableKind.Table }, 'c1', 'Sales', 'dbo')
    expect(table.kind).toBe('table')
    expect(table.key).toBe('c1/Sales/dbo/table/orders')
    expect(table.children).toEqual([])
    expect(table.loaded).toBe(false)

    const view = tableNode({ name: 'big', kind: TableKind.View }, 'c1', 'Sales', undefined)
    expect(view.kind).toBe('view')
    expect(view.key).toBe('c1/Sales//view/big')
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
        node({
          key: 'db',
          label: 'Sales',
          children: [node({ key: 't', label: 'orders', kind: 'table' })],
        }),
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
    walk([node({ key: 'a', children: [node({ key: 'b', children: undefined })] })], (visited) =>
      seen.push(visited.key),
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

  it('puts folders below a database when the engine has no schemas', async () => {
    const explorer = await readyStore(false)
    const database = node({ kind: 'database', database: 'shop', label: 'shop' })
    await explorer.expand(database)
    expect(database.children?.map((child) => child.label)).toEqual([
      'Tables',
      'Views',
      'Procedures',
      'Functions',
    ])
    expect(apiStub.listTables).not.toHaveBeenCalled()
  })

  it('puts folders below a schema', async () => {
    const explorer = await readyStore()
    const schema = node({ kind: 'schema', database: 'Sales', schema: 'dbo', label: 'dbo' })
    await explorer.expand(schema)
    expect(schema.children?.map((child) => child.folder)).toEqual([
      'tables',
      'views',
      'procedures',
      'functions',
    ])
  })

  it('leaves out the folders of an engine that holds no routine', async () => {
    apiStub.listActiveConnections.mockResolvedValue([
      {
        ...infoFixture('c1'),
        capabilities: { ...infoFixture('c1').capabilities, supportsRoutines: false },
      },
    ])
    const connections = useConnectionsStore()
    await connections.load()
    const explorer = useExplorerStore()
    const schema = node({ kind: 'schema', database: 'Sales', schema: 'dbo' })
    await explorer.expand(schema)
    expect(schema.children?.map((child) => child.folder)).toEqual(['tables', 'views'])
  })

  it('reads the tables and the views of their own folders', async () => {
    apiStub.listTables.mockResolvedValue([
      { name: 'orders', kind: TableKind.Table },
      { name: 'big_orders', kind: TableKind.View },
    ])
    const explorer = await readyStore()
    const schema = node({ kind: 'schema', database: 'Sales', schema: 'dbo' })

    const tables = folderNode('Tables', 'tables', schema)
    await explorer.expand(tables)
    expect(apiStub.listTables).toHaveBeenCalledWith('c1', 'Sales', 'dbo')
    expect(tables.children?.map((child) => child.label)).toEqual(['orders'])

    const views = folderNode('Views', 'views', schema)
    await explorer.expand(views)
    expect(views.children?.map((child) => child.label)).toEqual(['big_orders'])
    expect(views.children?.[0]?.kind).toBe('view')
  })

  it('uses an empty database name and no schema when the node carries none', async () => {
    apiStub.listTables.mockResolvedValue([])
    const explorer = await readyStore()
    const bare = node({ kind: 'schema', database: undefined, schema: undefined })
    await explorer.expand(folderNode('Tables', 'tables', bare))
    expect(apiStub.listTables).toHaveBeenCalledWith('c1', '', null)
  })

  it('reads the procedures and the functions of their own folders', async () => {
    apiStub.listRoutines.mockResolvedValue([
      { name: 'add_order', kind: 'procedure' },
      { name: 'order_total', kind: 'function' },
    ])
    const explorer = await readyStore()
    const schema = node({ kind: 'schema', database: 'Sales', schema: 'dbo' })

    const procedures = folderNode('Procedures', 'procedures', schema)
    await explorer.expand(procedures)
    expect(apiStub.listRoutines).toHaveBeenCalledWith('c1', 'Sales', 'dbo')
    expect(procedures.children?.map((child) => child.label)).toEqual(['add_order'])
    expect(procedures.children?.[0]?.kind).toBe('routine')

    const functions = folderNode('Functions', 'functions', schema)
    await explorer.expand(functions)
    expect(functions.children?.map((child) => child.label)).toEqual(['order_total'])
  })

  it('puts folders below a table and columns alone below a view', async () => {
    const explorer = await readyStore()
    const table = node({ kind: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })
    await explorer.expand(table)
    expect(table.children?.map((child) => child.folder)).toEqual([
      'columns',
      'indexes',
      'constraints',
    ])

    const view = node({ kind: 'view', database: 'Sales', schema: 'dbo', table: 'big_orders' })
    await explorer.expand(view)
    expect(view.children?.map((child) => child.folder)).toEqual(['columns'])
  })

  it('adds a folder for the partitions when the engine holds them', async () => {
    apiStub.listActiveConnections.mockResolvedValue([
      {
        ...infoFixture('c1'),
        capabilities: { ...infoFixture('c1').capabilities, supportsPartitions: true },
      },
    ])
    const connections = useConnectionsStore()
    await connections.load()
    const explorer = useExplorerStore()
    const table = node({ kind: 'table', database: 'logs', table: 'events' })
    await explorer.expand(table)
    expect(table.children?.map((child) => child.folder)).toEqual([
      'columns',
      'indexes',
      'constraints',
      'partitions',
    ])
  })

  it('reads the columns of the folder of a table', async () => {
    apiStub.listColumns.mockResolvedValue([
      { name: 'id', dataType: 'int', nullable: false, isPrimaryKey: true },
    ])
    const explorer = await readyStore()
    const table = node({ kind: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })
    const columns = folderNode('Columns', 'columns', table)
    await explorer.expand(columns)
    expect(apiStub.listColumns).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(columns.children?.[0]?.kind).toBe('column')
  })

  it('names the columns and the rule of each index and each constraint', async () => {
    apiStub.listIndexes.mockResolvedValue([
      { name: 'pk_orders', columns: ['id'], unique: true, primary: true },
      { name: 'by_region', columns: ['region'], unique: true, primary: false },
      { name: 'by_total', columns: ['total'], unique: false, primary: false },
    ])
    apiStub.listConstraints.mockResolvedValue([
      { name: 'pk_orders', kind: 'primaryKey', columns: ['id'], detail: null },
    ])
    apiStub.listPartitions.mockResolvedValue([{ values: 'day=2026-08-10' }])

    const explorer = await readyStore()
    const table = node({ kind: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })

    const indexes = folderNode('Indexes', 'indexes', table)
    await explorer.expand(indexes)
    expect(apiStub.listIndexes).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(indexes.children?.map((child) => child.hint)).toEqual([
      'id \u00b7 primary key',
      'region \u00b7 unique',
      'total',
    ])

    const keys = folderNode('Keys', 'constraints', table)
    await explorer.expand(keys)
    expect(keys.children?.[0]?.hint).toBe('primary key \u00b7 id')
    expect(keys.children?.[0]?.kind).toBe('constraint')

    const partitions = folderNode('Partitions', 'partitions', table)
    await explorer.expand(partitions)
    expect(apiStub.listPartitions).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(partitions.children?.[0]?.label).toBe('day=2026-08-10')
  })

  it('takes the name of a database from its label when it reads the schemas', async () => {
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    const explorer = await readyStore(true)
    await explorer.expand(node({ kind: 'database', label: 'Sales', database: undefined }))
    expect(apiStub.listSchemas).toHaveBeenCalledWith('c1', 'Sales')
  })

  it('leaves out the folders of a table that the engine cannot describe', async () => {
    apiStub.listActiveConnections.mockResolvedValue([
      {
        ...infoFixture('c1'),
        capabilities: {
          ...infoFixture('c1').capabilities,
          supportsIndexes: false,
          supportsConstraints: false,
        },
      },
    ])
    const connections = useConnectionsStore()
    await connections.load()
    const explorer = useExplorerStore()
    const table = node({ kind: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })
    await explorer.expand(table)
    expect(table.children?.map((child) => child.folder)).toEqual(['columns'])
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

  it('does not read a leaf, which holds nothing below it', async () => {
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
    const explorer = useExplorerStore()
    const database = node({ kind: 'database', database: 'shop', connectionId: 'other' })
    await explorer.expand(database)
    // The record of the connection is missing, so no folder of a capability
    // is added and no schema is read.
    expect(apiStub.listSchemas).not.toHaveBeenCalled()
    expect(database.children?.map((child) => child.folder)).toEqual(['tables', 'views'])
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
    const tables = schema.children![0]!
    await explorer.expand(tables)
    const table = tables.children![0]!
    await explorer.expand(table)
    const columns = table.children![0]!
    await explorer.expand(columns)

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
    // A second connection is still reading its own branch.
    const other = explorer.addRoot('c2')
    other.children = [node({ key: 'busy', loading: true, children: undefined })]
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
