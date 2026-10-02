import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { computed, isReactive, isShallow, nextTick, reactive } from 'vue'
import { makeApiStub, connectionFixture, infoFixture } from './helpers'
import { Dialect } from '@/types/api'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const {
  FILTER_DELAY_MS,
  columnNode,
  constraintHint,
  eventHint,
  indexColumns,
  filterNodes,
  folderNode,
  iconFor,
  isExpandable,
  isRelation,
  isTriggerOrEvent,
  leafNode,
  relationFoldersFor,
  tableNode,
  triggerHint,
  useExplorerStore,
  walk,
  withUniqueKeys,
} = await import('@/stores/explorer')
type ExplorerNode = import('@/stores/explorer').ExplorerNode
const { useConnectionsStore } = await import('@/stores/connections')
const { useUiStore } = await import('@/stores/ui')
const { RelationType } = await import('@/types/api')
const { emptySchemaIndex } = await import('@/lib/sql')

/** Waits for the pause that the filter of the tree holds. */
async function afterTheFilterPause(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, FILTER_DELAY_MS + 20))
}

function node(overrides: Partial<ExplorerNode> = {}): ExplorerNode {
  return {
    key: 'k',
    label: 'label',
    nodeType: 'database',
    icon: 'mdi-database',
    children: [],
    loading: false,
    loaded: false,
    connectionId: 'c1',
    ...overrides,
  }
}

describe('iconFor', () => {
  it('gives an icon for every type of node', () => {
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
    expect(iconFor('materializedView')).toBe('mdi-table-refresh')
    expect(iconFor('partitionedTable')).toBe('mdi-table-split-cell')
    expect(iconFor('foreignTable')).toBe('mdi-table-network')
    expect(iconFor('synonym')).toBe('mdi-link-variant')
    expect(iconFor('trigger')).toBe('mdi-lightning-bolt')
    expect(iconFor('event')).toBe('mdi-calendar-clock')
  })
})

describe('isExpandable', () => {
  it('holds for a node that can hold children', () => {
    expect(isExpandable(node({ nodeType: 'table' }))).toBe(true)
    expect(isExpandable(node({ nodeType: 'folder' }))).toBe(true)
    for (const nodeType of ['column', 'routine', 'index', 'constraint', 'partition'] as const) {
      expect(isExpandable(node({ nodeType }))).toBe(false)
    }
    // A synonym names another object, and the tree reads nothing below it.
    expect(isExpandable(node({ nodeType: 'synonym' }))).toBe(false)
    expect(isExpandable(node({ nodeType: 'materializedView' }))).toBe(true)
    expect(isExpandable(node({ nodeType: 'trigger' }))).toBe(false)
    expect(isExpandable(node({ nodeType: 'event' }))).toBe(false)
  })
})

describe('isTriggerOrEvent', () => {
  it('holds for a trigger and an event alone', () => {
    expect(isTriggerOrEvent(node({ nodeType: 'trigger' }))).toBe(true)
    expect(isTriggerOrEvent(node({ nodeType: 'event' }))).toBe(true)
    expect(isTriggerOrEvent(node({ nodeType: 'table' }))).toBe(false)
    expect(isTriggerOrEvent(node({ nodeType: 'routine' }))).toBe(false)
  })
})

describe('triggerHint and eventHint', () => {
  it('names the timing and the events of a trigger, and marks a disabled one', () => {
    const trigger: Parameters<typeof triggerHint>[0] = {
      name: 't',
      timing: 'after',
      events: ['insert', 'update'],
      enabled: true,
    }
    expect(triggerHint(trigger)).toBe('AFTER INSERT, UPDATE')
    expect(triggerHint({ ...trigger, timing: 'before', events: ['truncate'] })).toBe(
      'BEFORE TRUNCATE',
    )
    expect(
      triggerHint({ ...trigger, timing: 'insteadOf', events: ['delete'], enabled: false }),
    ).toBe('INSTEAD OF DELETE \u00b7 disabled')
    // A trigger whose events the engine did not report names its timing alone.
    expect(triggerHint({ ...trigger, events: [] })).toBe('AFTER')
  })

  it('names the schedule of an event, and marks a disabled one', () => {
    expect(eventHint({ name: 'e', enabled: true, schedule: 'EVERY 1 DAY' })).toBe('EVERY 1 DAY')
    expect(eventHint({ name: 'e', enabled: false, schedule: 'AT 2026-01-01' })).toBe(
      'AT 2026-01-01 \u00b7 disabled',
    )
    expect(eventHint({ name: 'e', enabled: false })).toBe('disabled')
    expect(eventHint({ name: 'e', enabled: true })).toBeUndefined()
  })
})

describe('isRelation', () => {
  it('holds for every type of relation and for no other node', () => {
    for (const nodeType of Object.values(RelationType)) {
      expect(isRelation(node({ nodeType }))).toBe(true)
    }
    for (const nodeType of ['connection', 'database', 'schema', 'folder', 'column'] as const) {
      expect(isRelation(node({ nodeType }))).toBe(false)
    }
  })
})

describe('relationFoldersFor', () => {
  it('adds the folder of each type of relation that the engine has, in the order of the tree', () => {
    const base = infoFixture().capabilities
    expect(relationFoldersFor(undefined)).toEqual(['tables', 'views'])
    expect(relationFoldersFor(base)).toEqual(['tables', 'views'])
    expect(
      relationFoldersFor({
        ...base,
        supportsMaterializedViews: true,
        supportsForeignTables: true,
        supportsSynonyms: true,
      }),
    ).toEqual(['tables', 'views', 'materializedViews', 'foreignTables', 'synonyms'])
  })
})

describe('folderNode and leafNode', () => {
  it('carries the place of the parent down to the child', () => {
    const table = node({
      nodeType: 'table',
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

describe('indexColumns', () => {
  it('names the key columns, and then the included columns', () => {
    const index = { name: 'cover', columns: ['a', 'b'], unique: false, primary: false }
    expect(indexColumns({ ...index, included: [] })).toBe('a, b')
    expect(indexColumns({ ...index, included: ['c', 'd'] })).toBe('a, b include (c, d)')
  })
})

describe('constraintHint', () => {
  it('names the type, the columns and the detail', () => {
    expect(
      constraintHint({ name: 'pk', constraintType: 'primaryKey', columns: ['id'], detail: null }),
    ).toBe('primary key \u00b7 id')
    expect(
      constraintHint({
        name: 'fk',
        constraintType: 'foreignKey',
        columns: ['customer'],
        detail: 'customers(id)',
      }),
    ).toBe('foreign key \u00b7 customer \u00b7 customers(id)')
    expect(constraintHint({ name: 'u', constraintType: 'unique', columns: [], detail: null })).toBe(
      'unique',
    )
    expect(
      constraintHint({ name: 'c', constraintType: 'check', columns: [], detail: 'total > 0' }),
    ).toBe('check \u00b7 total > 0')
    expect(
      constraintHint({ name: 'x', constraintType: 'exclusion', columns: ['room'], detail: null }),
    ).toBe('exclusion \u00b7 room')
    expect(
      constraintHint({ name: 't', constraintType: 'trigger', columns: [], detail: null }),
    ).toBe('trigger')
    expect(
      constraintHint({ name: 'n', constraintType: 'notNull', columns: ['id'], detail: null }),
    ).toBe('not null \u00b7 id')
    expect(
      constraintHint({
        name: 'df',
        constraintType: 'default',
        columns: ['made'],
        detail: '(getdate())',
      }),
    ).toBe('default \u00b7 made \u00b7 (getdate())')
  })
})

describe('tableNode', () => {
  it('builds a node for a table and one for a view', () => {
    const table = tableNode(
      { name: 'orders', relationType: RelationType.Table },
      'c1',
      'Sales',
      'dbo',
    )
    expect(table.nodeType).toBe('table')
    expect(table.key).toBe('c1/Sales/dbo/table/orders')
    expect(table.children).toEqual([])
    expect(table.loaded).toBe(false)

    const view = tableNode(
      { name: 'big', relationType: RelationType.View },
      'c1',
      'Sales',
      undefined,
    )
    expect(view.nodeType).toBe('view')
    expect(view.key).toBe('c1/Sales//view/big')

    const parted = tableNode(
      { name: 'events', relationType: RelationType.PartitionedTable },
      'c1',
      'logs',
      'public',
    )
    expect(parted.nodeType).toBe('partitionedTable')
    expect(parted.icon).toBe('mdi-table-split-cell')
    expect(parted.children).toEqual([])
  })

  it('builds a synonym as a leaf that names its target', () => {
    const synonym = tableNode(
      {
        name: 'remote_orders',
        relationType: RelationType.Synonym,
        target: '[Other].[dbo].[orders]',
      },
      'c1',
      'Sales',
      'dbo',
    )
    expect(synonym.nodeType).toBe('synonym')
    expect(synonym.key).toBe('c1/Sales/dbo/synonym/remote_orders')
    expect(synonym.hint).toBe('[Other].[dbo].[orders]')
    expect(synonym.children).toBeUndefined()
    expect(synonym.loaded).toBe(true)
    expect(synonym.table).toBe('remote_orders')

    const bare = tableNode(
      { name: 's', relationType: RelationType.Synonym, target: '' },
      'c1',
      'Sales',
      'dbo',
    )
    expect(bare.hint).toBeUndefined()
  })
})

describe('columnNode', () => {
  it('marks a key column and reports whether a column may hold no value', () => {
    const parent = node({ nodeType: 'table', key: 'c1/db/dbo/orders', table: 'orders' })
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
      nodeType: 'connection',
      children: [
        node({
          key: 'db',
          label: 'Sales',
          children: [node({ key: 't', label: 'orders', nodeType: 'table' })],
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

  it('keeps every child of a node that matches', () => {
    const filtered = filterNodes(tree, 'sales')
    expect(filtered[0]?.children?.[0]?.children?.map((child) => child.label)).toEqual(['orders'])
  })

  it('gives an empty list when nothing matches', () => {
    expect(filterNodes(tree, 'nothing')).toEqual([])
  })

  it('keeps a leaf without children as a leaf', () => {
    const leaves = [node({ key: 'c', label: 'id', nodeType: 'column', children: undefined })]
    expect(filterNodes(leaves, 'id')[0]?.children).toBeUndefined()
  })
})

describe('withUniqueKeys', () => {
  it('numbers each repeat of a key and keeps the first one', () => {
    const list = [node({ key: 'f/sum' }), node({ key: 'f/avg' }), node({ key: 'f/sum' })]
    expect(withUniqueKeys(list).map((each) => each.key)).toEqual(['f/sum', 'f/avg', 'f/sum#2'])
    expect(withUniqueKeys(list)[0]).toBe(list[0])
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

  it('expands the node of the tree itself when the filter hands out a copy', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')

    // The filter builds copies of the nodes, as the view sees them.
    explorer.filter = 'c1'
    const copy = { ...root, children: [] }
    await explorer.expand(copy)

    expect(root.children?.map((child) => child.label)).toEqual(['Sales'])
    expect(root.loaded).toBe(true)
  })

  it('reads the schemas below a database when the engine has them', async () => {
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    const explorer = await readyStore(true)
    const database = node({ nodeType: 'database', database: 'Sales', label: 'Sales' })
    await explorer.expand(database)
    expect(apiStub.listSchemas).toHaveBeenCalledWith('c1', 'Sales')
    expect(database.children?.[0]?.nodeType).toBe('schema')
  })

  it('puts folders below a SQLite database that has the schema main alone', async () => {
    apiStub.listActiveConnections.mockResolvedValue([
      { ...infoFixture('c1', true), dialect: Dialect.Sqlite },
    ])
    await useConnectionsStore().load()
    const explorer = useExplorerStore()
    apiStub.listSchemas.mockResolvedValue([{ name: 'main' }])
    const database = node({ nodeType: 'database', database: 'main', label: 'main' })
    await explorer.expand(database)
    expect(database.children?.[0]?.label).toBe('Tables')

    apiStub.listSchemas.mockResolvedValue([{ name: 'main' }, { name: 'temp' }])
    const attached = node({ nodeType: 'database', database: 'other', label: 'other' })
    await explorer.expand(attached)
    expect(attached.children?.map((child) => child.nodeType)).toEqual(['schema', 'schema'])
  })

  it('keeps a schema main of an engine other than SQLite', async () => {
    apiStub.listSchemas.mockResolvedValue([{ name: 'main' }])
    const explorer = await readyStore(true)
    const database = node({ nodeType: 'database', database: 'Sales', label: 'Sales' })
    await explorer.expand(database)
    expect(database.children?.[0]?.nodeType).toBe('schema')
  })

  it('puts folders below a database when the engine has no schemas', async () => {
    const explorer = await readyStore(false)
    const database = node({ nodeType: 'database', database: 'shop', label: 'shop' })
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
    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo', label: 'dbo' })
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
    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo' })
    await explorer.expand(schema)
    expect(schema.children?.map((child) => child.folder)).toEqual(['tables', 'views'])
  })

  /** Loads a store whose engine has every type of relation of its own. */
  async function storeWithEveryRelationType() {
    apiStub.listActiveConnections.mockResolvedValue([
      {
        ...infoFixture('c1'),
        capabilities: {
          ...infoFixture('c1').capabilities,
          supportsPartitions: true,
          supportsMaterializedViews: true,
          supportsForeignTables: true,
          supportsSynonyms: true,
        },
      },
    ])
    const connections = useConnectionsStore()
    await connections.load()
    return useExplorerStore()
  }

  it('puts the folders of the types of relation that the engine has between the views and the routines', async () => {
    const explorer = await storeWithEveryRelationType()
    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo' })
    await explorer.expand(schema)
    expect(schema.children?.map((child) => [child.folder, child.label])).toEqual([
      ['tables', 'Tables'],
      ['views', 'Views'],
      ['materializedViews', 'Materialized Views'],
      ['foreignTables', 'Foreign Tables'],
      ['synonyms', 'Synonyms'],
      ['procedures', 'Procedures'],
      ['functions', 'Functions'],
    ])
  })

  it('sorts each type of relation into its folder from one shared read', async () => {
    apiStub.listTables.mockResolvedValue([
      { name: 'orders', relationType: RelationType.Table },
      { name: 'events', relationType: RelationType.PartitionedTable },
      { name: 'big_orders', relationType: RelationType.View },
      { name: 'totals', relationType: RelationType.MaterializedView },
      { name: 'remote', relationType: RelationType.ForeignTable },
      { name: 'alias', relationType: RelationType.Synonym, target: 'other.dbo.orders' },
    ])
    const explorer = await storeWithEveryRelationType()
    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo' })
    const read = async (folder: Parameters<typeof folderNode>[1]) => {
      const holder = folderNode(folder, folder, schema)
      await explorer.expand(holder)
      return holder.children?.map((child) => [child.label, child.nodeType])
    }

    expect(await read('tables')).toEqual([
      ['orders', 'table'],
      ['events', 'partitionedTable'],
    ])
    expect(await read('views')).toEqual([['big_orders', 'view']])
    expect(await read('materializedViews')).toEqual([['totals', 'materializedView']])
    expect(await read('foreignTables')).toEqual([['remote', 'foreignTable']])
    // The read stays shared until the last of the five folders reads it.
    expect(apiStub.listTables).toHaveBeenCalledTimes(1)
    expect(await read('synonyms')).toEqual([['alias', 'synonym']])
    expect(apiStub.listTables).toHaveBeenCalledTimes(1)

    await read('tables')
    expect(apiStub.listTables).toHaveBeenCalledTimes(2)
  })

  it('puts the folders of its type below each new type of relation', async () => {
    const explorer = await storeWithEveryRelationType()
    const place = { database: 'Sales', schema: 'public' }
    const foldersOf = async (nodeType: ExplorerNode['nodeType']) => {
      const relation = node({ nodeType, ...place, table: 'r', key: nodeType })
      await explorer.expand(relation)
      return relation.children?.map((child) => child.folder)
    }
    expect(await foldersOf('partitionedTable')).toEqual([
      'columns',
      'indexes',
      'constraints',
      'partitions',
    ])
    expect(await foldersOf('materializedView')).toEqual(['columns', 'indexes'])
    expect(await foldersOf('foreignTable')).toEqual(['columns', 'constraints'])
  })

  it('offers the name of a synonym to the editor', async () => {
    apiStub.listTables.mockResolvedValue([
      { name: 'alias', relationType: RelationType.Synonym, target: 'other.dbo.orders' },
    ])
    const explorer = await storeWithEveryRelationType()
    const root = explorer.addRoot('c1')
    const schema = node({
      key: 'c1/Sales/dbo',
      nodeType: 'schema',
      database: 'Sales',
      schema: 'dbo',
      label: 'dbo',
    })
    root.children = [schema]
    root.loaded = true
    await explorer.expand(schema)
    const synonyms = schema.children!.find((child) => child.folder === 'synonyms')!
    await explorer.expand(synonyms)
    expect(explorer.schemaIndexFor('c1').tables).toEqual([
      { name: 'alias', qualifier: 'Sales.dbo' },
    ])
  })

  it('reads the tables and the views of their own folders', async () => {
    apiStub.listTables.mockResolvedValue([
      { name: 'orders', relationType: RelationType.Table },
      { name: 'big_orders', relationType: RelationType.View },
    ])
    const explorer = await readyStore()
    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo' })

    const tables = folderNode('Tables', 'tables', schema)
    await explorer.expand(tables)
    expect(apiStub.listTables).toHaveBeenCalledWith('c1', 'Sales', 'dbo')
    expect(tables.children?.map((child) => child.label)).toEqual(['orders'])

    const views = folderNode('Views', 'views', schema)
    await explorer.expand(views)
    expect(views.children?.map((child) => child.label)).toEqual(['big_orders'])
    expect(views.children?.[0]?.nodeType).toBe('view')
  })

  it('shares one read of the relations between the two folders of a schema', async () => {
    apiStub.listTables.mockResolvedValue([
      { name: 'orders', relationType: RelationType.Table },
      { name: 'big_orders', relationType: RelationType.View },
    ])
    const explorer = await readyStore()
    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo' })
    await explorer.expand(folderNode('Tables', 'tables', schema))
    const views = folderNode('Views', 'views', schema)
    await explorer.expand(views)
    expect(apiStub.listTables).toHaveBeenCalledTimes(1)
    expect(views.children?.map((child) => child.label)).toEqual(['big_orders'])

    // Both folders have read the list, so the next read asks the backend.
    await explorer.expand(folderNode('Tables', 'tables', schema))
    expect(apiStub.listTables).toHaveBeenCalledTimes(2)
  })

  it('drops a shared read of relations on a refresh, a close and a failure', async () => {
    apiStub.listTables.mockResolvedValue([])
    const explorer = await readyStore()
    const place = (connectionId: string, database: string, schema?: string) =>
      node({
        nodeType: 'schema',
        key: `${connectionId}/${database}/${schema}`,
        connectionId,
        database,
        schema,
      })
    const places = [
      place('c1', 'Sales', 'dbo'),
      place('c1', 'Sales', 'stage'),
      place('c1', 'Other', 'dbo'),
      place('c2', 'Sales', 'dbo'),
    ]
    // The Tables folder of each place reads, and each read stays for Views.
    for (const schema of places) {
      await explorer.expand(folderNode('Tables', 'tables', schema))
    }
    expect(apiStub.listTables).toHaveBeenCalledTimes(4)

    // A refresh of one schema drops the read of that schema alone.
    await explorer.refresh(places[0]!)
    for (const schema of places) {
      await explorer.expand(folderNode('Views', 'views', schema))
    }
    expect(apiStub.listTables).toHaveBeenCalledTimes(5)

    for (const schema of places) {
      await explorer.expand(folderNode('Tables', 'tables', { ...schema, key: `${schema.key}!` }))
    }
    expect(apiStub.listTables).toHaveBeenCalledTimes(8)

    // A close drops every read of the connection, and the read of the other
    // connection stays shared.
    explorer.removeRoot('c1')
    for (const schema of places) {
      await explorer.expand(folderNode('Views', 'views', { ...schema, key: `${schema.key}?` }))
    }
    expect(apiStub.listTables).toHaveBeenCalledTimes(11)

    // clear drops every read.
    explorer.clear()
    for (const schema of places.slice(1, 3)) {
      await explorer.expand(folderNode('Tables', 'tables', { ...schema, key: `${schema.key}+` }))
    }
    expect(apiStub.listTables).toHaveBeenCalledTimes(13)

    // A read that fails is not shared, so the other folder asks again.
    apiStub.listTables.mockRejectedValueOnce({
      category: 'database',
      message: 'gone',
      detail: null,
    })
    const fresh = place('c1', 'Fresh', 'dbo')
    await explorer.expand(folderNode('Tables', 'tables', fresh))
    await explorer.expand(folderNode('Views', 'views', fresh))
    expect(apiStub.listTables).toHaveBeenCalledTimes(15)
  })

  it('keeps the shared read that took the place of a read that failed', async () => {
    const explorer = await readyStore()
    let fail: (reason: unknown) => void = () => {}
    apiStub.listTables.mockImplementationOnce(
      () =>
        new Promise((_, reject) => {
          fail = reject
        }),
    )
    apiStub.listTables.mockResolvedValue([])
    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo' })
    const first = explorer.expand(folderNode('Tables', 'tables', schema))
    await explorer.refresh(schema)
    await explorer.expand(folderNode('Views', 'views', schema))
    fail({ category: 'database', message: 'gone', detail: null })
    await first
    await explorer.expand(folderNode('Tables', 'tables', { ...schema, key: 'other' }))
    expect(apiStub.listTables).toHaveBeenCalledTimes(2)
  })

  it('drops the shared reads of a whole connection on a refresh of its root', async () => {
    apiStub.listTables.mockResolvedValue([])
    apiStub.listDatabases.mockResolvedValue([])
    const explorer = await readyStore()
    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo' })
    await explorer.expand(folderNode('Tables', 'tables', schema))
    await explorer.refresh(explorer.addRoot('c1'))
    await explorer.expand(folderNode('Views', 'views', schema))
    expect(apiStub.listTables).toHaveBeenCalledTimes(2)
  })

  it('uses an empty database name and no schema when the node carries none', async () => {
    apiStub.listTables.mockResolvedValue([])
    const explorer = await readyStore()
    const bare = node({ nodeType: 'schema', database: undefined, schema: undefined })
    await explorer.expand(folderNode('Tables', 'tables', bare))
    expect(apiStub.listTables).toHaveBeenCalledWith('c1', '', null)
  })

  it('reads the procedures and the functions of their own folders', async () => {
    apiStub.listRoutines.mockResolvedValue([
      { name: 'add_order', routineType: 'procedure' },
      { name: 'order_total', routineType: 'function' },
      { name: 'order_total', routineType: 'function' },
    ])
    const explorer = await readyStore()
    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo' })

    const procedures = folderNode('Procedures', 'procedures', schema)
    await explorer.expand(procedures)
    expect(apiStub.listRoutines).toHaveBeenCalledWith('c1', 'Sales', 'dbo')
    expect(procedures.children?.map((child) => child.label)).toEqual(['add_order'])
    expect(procedures.children?.[0]?.nodeType).toBe('routine')

    const functions = folderNode('Functions', 'functions', schema)
    await explorer.expand(functions)
    expect(functions.children?.map((child) => child.label)).toEqual(['order_total', 'order_total'])
    // Two overloads of one function get two keys.
    expect(new Set(functions.children?.map((child) => child.key)).size).toBe(2)
  })

  it('puts folders below a table and columns alone below a view', async () => {
    const explorer = await readyStore()
    const table = node({ nodeType: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })
    await explorer.expand(table)
    expect(table.children?.map((child) => child.folder)).toEqual([
      'columns',
      'indexes',
      'constraints',
    ])

    const view = node({ nodeType: 'view', database: 'Sales', schema: 'dbo', table: 'big_orders' })
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
    const table = node({ nodeType: 'table', database: 'logs', table: 'events' })
    await explorer.expand(table)
    expect(table.children?.map((child) => child.folder)).toEqual([
      'columns',
      'indexes',
      'constraints',
      'partitions',
    ])
  })

  /** Loads a store whose engine has triggers, and events when asked. */
  async function storeWithTriggers(supportsViewTriggers: boolean, supportsEvents = false) {
    apiStub.listActiveConnections.mockResolvedValue([
      {
        ...infoFixture('c1'),
        capabilities: {
          ...infoFixture('c1').capabilities,
          supportsPartitions: true,
          supportsMaterializedViews: true,
          supportsForeignTables: true,
          supportsTriggers: true,
          supportsViewTriggers,
          supportsEvents,
        },
      },
    ])
    const connections = useConnectionsStore()
    await connections.load()
    return useExplorerStore()
  }

  it('puts a folder of triggers below each relation that can have them', async () => {
    const explorer = await storeWithTriggers(true)
    const foldersOf = async (relationType: ExplorerNode['nodeType']) => {
      const relation = node({
        nodeType: relationType,
        database: 'Sales',
        schema: 'dbo',
        table: 'r',
        key: relationType,
      })
      await explorer.expand(relation)
      return relation.children?.map((child) => child.folder)
    }
    expect(await foldersOf('table')).toEqual([
      'columns',
      'indexes',
      'constraints',
      'triggers',
      'partitions',
    ])
    expect(await foldersOf('foreignTable')).toEqual(['columns', 'constraints', 'triggers'])
    expect(await foldersOf('view')).toEqual(['columns', 'triggers'])
    expect(await foldersOf('materializedView')).toEqual(['columns', 'indexes'])
  })

  it('leaves the folder of triggers out of a view on an engine without view triggers', async () => {
    const explorer = await storeWithTriggers(false)
    const view = node({ nodeType: 'view', database: 'shop', table: 'v' })
    await explorer.expand(view)
    expect(view.children?.map((child) => child.folder)).toEqual(['columns'])
  })

  it('puts the folder of events after the routines', async () => {
    const explorer = await storeWithTriggers(false, true)
    const database = node({ nodeType: 'schema', database: 'shop', schema: 'dbo' })
    await explorer.expand(database)
    expect(database.children?.map((child) => child.label).slice(-3)).toEqual([
      'Procedures',
      'Functions',
      'Events',
    ])
  })

  it('names the timing and the events of each trigger and dims a disabled one', async () => {
    apiStub.listTriggers.mockResolvedValue([
      { name: 'audit', timing: 'after', events: ['insert', 'update'], enabled: true },
      { name: 'old', timing: 'insteadOf', events: ['delete'], enabled: false },
    ])
    apiStub.listEvents.mockResolvedValue([
      { name: 'nightly', enabled: true, schedule: 'EVERY 1 DAY' },
      { name: 'paused', enabled: false },
    ])
    const explorer = await readyStore()
    const table = node({ nodeType: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })
    const triggers = folderNode('Triggers', 'triggers', table)
    await explorer.expand(triggers)
    expect(apiStub.listTriggers).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(
      triggers.children?.map((child) => [child.label, child.nodeType, child.hint, child.dimmed]),
    ).toEqual([
      ['audit', 'trigger', 'AFTER INSERT, UPDATE', false],
      ['old', 'trigger', 'INSTEAD OF DELETE \u00b7 disabled', true],
    ])
    expect(triggers.children?.[0]?.table).toBe('orders')

    const schema = node({ nodeType: 'schema', database: 'Sales', schema: 'dbo' })
    const events = folderNode('Events', 'events', schema)
    await explorer.expand(events)
    expect(apiStub.listEvents).toHaveBeenCalledWith('c1', 'Sales', 'dbo')
    expect(
      events.children?.map((child) => [child.label, child.nodeType, child.hint, child.dimmed]),
    ).toEqual([
      ['nightly', 'event', 'EVERY 1 DAY', false],
      ['paused', 'event', 'disabled', true],
    ])
  })

  it('reads the columns of the folder of a table', async () => {
    apiStub.listColumns.mockResolvedValue([
      { name: 'id', dataType: 'int', nullable: false, isPrimaryKey: true },
    ])
    const explorer = await readyStore()
    const table = node({ nodeType: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })
    const columns = folderNode('Columns', 'columns', table)
    await explorer.expand(columns)
    expect(apiStub.listColumns).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(columns.children?.[0]?.nodeType).toBe('column')
  })

  it('names the columns and the rule of each index and each constraint', async () => {
    apiStub.listIndexes.mockResolvedValue([
      { name: 'pk_orders', columns: ['id'], unique: true, primary: true, included: [] },
      { name: 'by_region', columns: ['region'], unique: true, primary: false, included: [] },
      { name: 'by_total', columns: ['total'], unique: false, primary: false, included: [] },
    ])
    apiStub.listConstraints.mockResolvedValue([
      { name: 'pk_orders', constraintType: 'primaryKey', columns: ['id'], detail: null },
    ])
    apiStub.listPartitions.mockResolvedValue({
      partitions: [{ values: 'day=2026-08-10' }],
      truncated: false,
    })

    const explorer = await readyStore()
    const table = node({ nodeType: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })

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
    expect(keys.children?.[0]?.nodeType).toBe('constraint')

    const partitions = folderNode('Partitions', 'partitions', table)
    await explorer.expand(partitions)
    expect(apiStub.listPartitions).toHaveBeenCalledWith('c1', 'Sales', 'dbo', 'orders')
    expect(partitions.children?.[0]?.label).toBe('day=2026-08-10')
    expect(partitions.hint).toBeUndefined()
  })

  it('marks a list of partitions that stopped at the limit of the read', async () => {
    apiStub.listPartitions.mockResolvedValue({
      partitions: [{ values: 'day=1' }, { values: 'day=2' }],
      truncated: true,
    })
    const explorer = await readyStore()
    const table = node({ nodeType: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })
    const partitions = folderNode('Partitions', 'partitions', table)
    await explorer.expand(partitions)
    expect(partitions.hint).toBe('first 2')
  })

  it('takes the name of a database from its label when it reads the schemas', async () => {
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    const explorer = await readyStore(true)
    await explorer.expand(node({ nodeType: 'database', label: 'Sales', database: undefined }))
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
    const table = node({ nodeType: 'table', database: 'Sales', schema: 'dbo', table: 'orders' })
    await explorer.expand(table)
    expect(table.children?.map((child) => child.folder)).toEqual(['columns'])
  })

  function snapshotFixture(database = 'Sales') {
    return {
      database,
      relations: [
        {
          name: 'orders',
          schema: 'dbo',
          relationType: RelationType.Table,
          columns: [{ name: 'id', dataType: 'int' }],
        },
        {
          name: 'orders',
          schema: 'staging',
          relationType: RelationType.View,
          columns: [{ name: 'raw', dataType: 'text' }],
        },
      ],
      columnCount: 2,
      complete: true,
    }
  }

  it('reads the schema of a database and keeps it', async () => {
    apiStub.schemaSnapshot.mockResolvedValue(snapshotFixture())
    const explorer = await readyStore()
    const options = { maxColumns: 100, ownConnection: true }

    await explorer.readSnapshot('c1', 'Sales', options)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledWith({
      connectionId: 'c1',
      database: 'Sales',
      maxColumns: 100,
      ownConnection: true,
    })

    // A second call reads nothing again.
    await explorer.readSnapshot('c1', 'Sales', options)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(1)

    // A call that asks for a fresh read makes one.
    await explorer.readSnapshot('c1', 'Sales', options, true)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(2)
  })

  it('makes one call for two reads of one database that run at the same time', async () => {
    const explorer = await readyStore()
    const answers: ((value: unknown) => void)[] = []
    apiStub.schemaSnapshot.mockImplementation(() => new Promise((resolve) => answers.push(resolve)))
    const options = { maxColumns: 100, ownConnection: true }

    const first = explorer.readSnapshot('c1', 'Sales', options)
    const second = explorer.readSnapshot('c1', 'Sales', options)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(1)
    answers[0]!(snapshotFixture())
    expect(await second).toEqual(snapshotFixture())
    expect(await first).toEqual(snapshotFixture())

    // The read is over, so the next read gives the kept snapshot.
    await explorer.readSnapshot('c1', 'Sales', options)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(1)
  })

  it('keeps the answer of a forced read over the answer of an older read', async () => {
    const explorer = await readyStore()
    const answers: ((value: unknown) => void)[] = []
    apiStub.schemaSnapshot.mockImplementation(() => new Promise((resolve) => answers.push(resolve)))
    const options = { maxColumns: 100, ownConnection: true }

    const older = explorer.readSnapshot('c1', 'Sales', options)
    const forced = explorer.readSnapshot('c1', 'Sales', options, true)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(2)
    // A third caller joins the forced read.
    const joined = explorer.readSnapshot('c1', 'Sales', options)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(2)

    answers[1]!({ ...snapshotFixture(), columnCount: 9 })
    await forced
    expect((await joined)?.columnCount).toBe(9)
    answers[0]!(snapshotFixture())
    expect((await older)?.columnCount).toBe(2)
    expect(explorer.snapshots['c1/Sales']?.columnCount).toBe(9)
  })

  it('starts a new read after the snapshots of the connection are dropped', async () => {
    const explorer = await readyStore()
    const answers: ((value: unknown) => void)[] = []
    apiStub.schemaSnapshot.mockImplementation(() => new Promise((resolve) => answers.push(resolve)))
    const options = { maxColumns: 100, ownConnection: true }

    const before = explorer.readSnapshot('c1', 'Sales', options)
    const other = explorer.readSnapshot('c2', 'Sales', options)
    explorer.forgetSnapshots('c1')
    const after = explorer.readSnapshot('c1', 'Sales', options)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(3)
    // The read of another connection goes on.
    void explorer.readSnapshot('c2', 'Sales', options)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(3)
    explorer.clear()
    const cleared = explorer.readSnapshot('c2', 'Sales', options)
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(4)
    for (const answer of answers) {
      answer(snapshotFixture())
    }
    expect(await before).toBe(null)
    expect(await after).toBe(null)
    expect(await other).toBe(null)
    expect(await cleared).toEqual(snapshotFixture())
  })

  it('reads the schema again when the user refreshes the tree', async () => {
    apiStub.schemaSnapshot.mockResolvedValue(snapshotFixture())
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'Shut' }])
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    const sales = root.children![0]!
    await explorer.expand(sales)
    await explorer.readSnapshot('c1', 'Shut', { maxColumns: 100, ownConnection: true })
    await Promise.resolve()
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(2)

    // A refresh of the database reads its schema again.
    await explorer.refresh(sales)
    await Promise.resolve()
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(3)

    // A refresh of the connection drops every schema and reads the open ones.
    await explorer.refresh(root, new Set([root.key, sales.key]))
    await Promise.resolve()
    expect(apiStub.schemaSnapshot).toHaveBeenCalledTimes(4)
    expect(Object.keys(explorer.snapshots)).toEqual(['c1/Sales'])
  })

  it('offers the names of a snapshot, and tells two schemas apart', async () => {
    apiStub.schemaSnapshot.mockResolvedValue(snapshotFixture())
    const explorer = await readyStore()
    await explorer.readSnapshot('c1', 'Sales', { maxColumns: 100, ownConnection: true })

    expect(explorer.schemaIndexFor('c1').databases).toEqual(['Sales'])
    expect(explorer.schemaIndexFor('c1').schemas).toEqual(['dbo', 'staging'])
    expect(explorer.schemaIndexFor('c1').tables).toEqual([
      { name: 'orders', qualifier: 'Sales.dbo' },
      { name: 'orders', qualifier: 'Sales.staging' },
    ])
    expect(explorer.schemaIndexFor('c1').columns).toEqual([
      { name: 'id', table: 'orders', qualifier: 'Sales.dbo', dataType: 'int' },
      { name: 'raw', table: 'orders', qualifier: 'Sales.staging', dataType: 'text' },
    ])
  })

  it('rebuilds the index of a large schema and reports the time', async () => {
    const relations = Array.from({ length: 200 }, (_, table) => ({
      name: `table_${table}`,
      schema: 'dbo',
      relationType: RelationType.Table,
      columns: Array.from({ length: 100 }, (_, column) => ({
        name: `column_${table}_${column}`,
        dataType: 'int',
      })),
    }))
    apiStub.schemaSnapshot.mockResolvedValue({
      database: 'Sales',
      relations,
      columnCount: 20_000,
      complete: true,
    })
    const explorer = await readyStore()
    await explorer.readSnapshot('c1', 'Sales', { maxColumns: 20_000, ownConnection: true })

    const firstStart = performance.now()
    expect(explorer.schemaIndexFor('c1').columns).toHaveLength(20_000)
    const fromSnapshot = performance.now() - firstStart

    // A new root invalidates the index, so the next read walks the tree
    // and the snapshot again.
    explorer.addRoot('c1')
    const secondStart = performance.now()
    expect(explorer.schemaIndexFor('c1').tables).toHaveLength(200)
    const afterTreeChange = performance.now() - secondStart

    console.warn(
      `schemaIndex with 20000 columns: ${fromSnapshot.toFixed(1)} ms from the snapshot, ` +
        `${afterTreeChange.toFixed(1)} ms after a change of the tree`,
    )
    expect(explorer.schemaIndexFor('c1').databases).toEqual(['Sales'])
    expect(explorer.schemaIndexFor('c1').schemas).toEqual(['dbo'])
  })

  it('warns when the bound stopped the read of a schema', async () => {
    apiStub.schemaSnapshot.mockResolvedValue({ ...snapshotFixture(), complete: false })
    const explorer = await readyStore()
    await explorer.readSnapshot('c1', 'Sales', { maxColumns: 1, ownConnection: false })
    expect(useUiStore().notices[0]?.level).toBe('warning')
  })

  it('reports a schema that cannot be read and keeps nothing', async () => {
    apiStub.schemaSnapshot.mockRejectedValue({ category: 'database', message: 'no', detail: null })
    const explorer = await readyStore()
    const answer = await explorer.readSnapshot('c1', 'Sales', {
      maxColumns: 10,
      ownConnection: true,
    })
    expect(answer).toBe(null)
    expect(explorer.snapshots).toEqual({})
    expect(useUiStore().notices[0]?.level).toBe('error')
  })

  it('leaves out an answer that is not a snapshot', async () => {
    apiStub.schemaSnapshot.mockResolvedValue(undefined)
    const explorer = await readyStore()
    const answer = await explorer.readSnapshot('c1', 'Sales', {
      maxColumns: 10,
      ownConnection: true,
    })
    expect(answer).toBe(null)
    expect(explorer.snapshots).toEqual({})
  })

  it('reads the schema when the user opens a database, and forgets it later', async () => {
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    apiStub.schemaSnapshot.mockResolvedValue(snapshotFixture())
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(node({ nodeType: 'database', database: 'Sales', label: 'Sales' }))
    expect(apiStub.schemaSnapshot).toHaveBeenCalledWith({
      connectionId: 'c1',
      database: 'Sales',
      maxColumns: 20000,
      ownConnection: true,
    })

    explorer.removeRoot(root.connectionId)
    expect(explorer.snapshots).toEqual({})
  })

  it('keeps the snapshots of the other connections', async () => {
    apiStub.schemaSnapshot.mockResolvedValue(snapshotFixture())
    const explorer = await readyStore()
    await explorer.readSnapshot('c1', 'Sales', { maxColumns: 10, ownConnection: true })
    await explorer.readSnapshot('c2', 'Other', { maxColumns: 10, ownConnection: true })
    explorer.forgetSnapshots('c1')
    expect(Object.keys(explorer.snapshots)).toEqual(['c2/Other'])

    explorer.clear()
    expect(explorer.snapshots).toEqual({})
  })

  it('offers the names of one connection alone', async () => {
    const explorer = await readyStore()
    apiStub.schemaSnapshot.mockResolvedValue(snapshotFixture('Sales'))
    await explorer.readSnapshot('c1', 'Sales', { maxColumns: 10, ownConnection: true })
    apiStub.schemaSnapshot.mockResolvedValue(snapshotFixture('Other'))
    await explorer.readSnapshot('c2', 'Other', { maxColumns: 10, ownConnection: true })
    explorer.roots = [
      node({ key: 'c1', nodeType: 'connection', label: 'One', connectionId: 'c1' }),
      node({ key: 'c2', nodeType: 'connection', label: 'Two', connectionId: 'c2' }),
    ]
    explorer.roots[1]!.children = [
      node({ key: 'c2/Archive', label: 'Archive', database: 'Archive', connectionId: 'c2' }),
    ]

    expect(explorer.schemaIndexFor('c1').databases).toEqual(['Sales'])
    expect(explorer.schemaIndexFor('c2').databases).toEqual(['Other', 'Archive'])
    expect(explorer.schemaIndexFor(null)).toEqual(emptySchemaIndex())

    explorer.forgetSnapshots('c2')
    expect(explorer.schemaIndexFor('c2').databases).toEqual(['Archive'])
    explorer.clear()
    expect(explorer.schemaIndexFor('c1')).toEqual(emptySchemaIndex())
  })

  it('drops a snapshot whose connection closed during the read', async () => {
    const explorer = await readyStore()
    let answer: (value: unknown) => void = () => {}
    apiStub.schemaSnapshot.mockImplementation(
      () =>
        new Promise((resolve) => {
          answer = resolve
        }),
    )
    const options = { maxColumns: 10, ownConnection: true }

    const read = explorer.readSnapshot('c1', 'Sales', options)
    explorer.forgetSnapshots('c1')
    answer(snapshotFixture())
    expect(await read).toBe(null)
    expect(explorer.snapshots).toEqual({})

    const second = explorer.readSnapshot('c1', 'Sales', options)
    explorer.clear()
    answer(snapshotFixture())
    expect(await second).toBe(null)
    expect(explorer.snapshots).toEqual({})
  })

  it('names the bounds the settings hold', async () => {
    const explorer = await readyStore()
    expect(explorer.snapshotOptions()).toEqual({ maxColumns: 20000, ownConnection: true })
  })

  it('holds one record for a relation that two reads both name', async () => {
    const explorer = await readyStore()
    apiStub.schemaSnapshot.mockResolvedValue(snapshotFixture())
    await explorer.readSnapshot('c1', 'Sales', { maxColumns: 10, ownConnection: true })
    // The same database read again under another key gives the same names.
    apiStub.schemaSnapshot.mockResolvedValue(snapshotFixture())
    await explorer.readSnapshot('c1', 'Sales2', { maxColumns: 10, ownConnection: true })
    expect(explorer.schemaIndexFor('c1').tables).toEqual([
      { name: 'orders', qualifier: 'Sales.dbo' },
      { name: 'orders', qualifier: 'Sales.staging' },
    ])
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
    await explorer.expand(node({ nodeType: 'column' }))
    expect(apiStub.listColumns).not.toHaveBeenCalled()
  })

  it('reports a failure and leaves the branch closed', async () => {
    apiStub.listDatabases.mockRejectedValue({
      category: 'notConnected',
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

  it('reads the open branches below a refreshed node again', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'Shut' }])
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    const explorer = await readyStore(true)
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    const sales = root.children![0]!
    await explorer.expand(sales)
    const open = new Set([root.key, sales.key, `${sales.key}/dbo`])

    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }, { name: 'audit' }])
    await explorer.refresh(root, open)
    const fresh = root.children![0]!
    expect(fresh).not.toBe(sales)
    expect(fresh.children?.map((child) => child.label)).toEqual(['dbo', 'audit'])
    // A branch that the tree shows closed waits for its own expand.
    expect(root.children![1]!.loaded).toBe(false)
  })

  it('drops the answer of a read that a refresh passed', async () => {
    let releaseFirst: (value: { name: string }[]) => void = () => {}
    apiStub.listDatabases.mockReturnValueOnce(
      new Promise<{ name: string }[]>((resolve) => {
        releaseFirst = resolve
      }),
    )
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    const first = explorer.expand(root)

    apiStub.listDatabases.mockResolvedValue([{ name: 'New' }])
    await explorer.refresh(root)
    expect(root.children?.map((child) => child.label)).toEqual(['New'])
    expect(root.loading).toBe(false)

    releaseFirst([{ name: 'Old' }])
    await first
    // The older answer holds no place, so the newer one stays.
    expect(root.children?.map((child) => child.label)).toEqual(['New'])
    expect(root.loaded).toBe(true)
    expect(root.loading).toBe(false)
  })

  it('says nothing about a failure of a read that a refresh passed', async () => {
    let refuseFirst: (error: unknown) => void = () => {}
    apiStub.listDatabases.mockReturnValueOnce(
      new Promise<{ name: string }[]>((_resolve, reject) => {
        refuseFirst = reject
      }),
    )
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    const first = explorer.expand(root)

    apiStub.listDatabases.mockResolvedValue([{ name: 'New' }])
    await explorer.refresh(root)

    refuseFirst({ category: 'notConnected', message: 'gone', detail: null })
    await first
    expect(useUiStore().notices).toEqual([])
    expect(root.children?.map((child) => child.label)).toEqual(['New'])
    expect(root.loaded).toBe(true)
  })

  it('reads nothing again for a leaf', async () => {
    const explorer = await readyStore()
    await explorer.refresh(node({ nodeType: 'column' }))
    expect(apiStub.listColumns).not.toHaveBeenCalled()
  })

  it('treats a connection the store does not know as one without schemas', async () => {
    const explorer = useExplorerStore()
    const database = node({ nodeType: 'database', database: 'shop', connectionId: 'other' })
    await explorer.expand(database)
    // The record of the connection is missing, so no folder of a capability
    // is added and no schema is read.
    expect(apiStub.listSchemas).not.toHaveBeenCalled()
    expect(database.children?.map((child) => child.folder)).toEqual(['tables', 'views'])
  })

  it('builds the names the editor offers, without repeating one', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }])
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    apiStub.listTables.mockResolvedValue([{ name: 'orders', relationType: RelationType.Table }])
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

    expect(explorer.schemaIndexFor('c1')).toEqual({
      databases: ['Sales'],
      schemas: ['dbo'],
      tables: [{ name: 'orders', qualifier: 'Sales.dbo' }],
      columns: [{ name: 'id', table: 'orders', qualifier: 'Sales.dbo', dataType: 'int not null' }],
    })

    // A second root over the same names adds nothing new.
    explorer.roots = [...explorer.roots, ...explorer.roots]
    expect(explorer.schemaIndexFor('c1').databases).toEqual(['Sales'])
  })

  it('reports a column without a type as one without a hint', async () => {
    const explorer = useExplorerStore()
    explorer.roots = [node({ nodeType: 'column', label: 'id', hint: undefined, table: undefined })]
    expect(explorer.schemaIndexFor('c1').columns).toEqual([
      { name: 'id', table: '', qualifier: '', dataType: '' },
    ])
  })

  it('keeps the reading flag while another branch still reads', async () => {
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    // A second connection is still reading its own branch.
    const other = explorer.addRoot('c2')
    let answer: (value: unknown) => void = () => {}
    apiStub.listDatabases.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve
        }),
    )
    const busy = explorer.expand(other)
    apiStub.listDatabases.mockResolvedValue([])
    await explorer.expand(root)
    expect(explorer.loading).toBe(true)
    answer([])
    await busy
    expect(explorer.loading).toBe(false)
  })

  it('drops the reading flag of a branch that left the tree', async () => {
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    const other = explorer.addRoot('c2')
    apiStub.listDatabases.mockImplementationOnce(() => new Promise(() => {}))
    void explorer.expand(other)
    explorer.removeRoot('c2')
    apiStub.listDatabases.mockResolvedValue([])
    await explorer.expand(root)
    expect(explorer.loading).toBe(false)
  })

  it('keeps a shallow tree that the view still follows', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'Other' }])
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    const explorer = await readyStore()
    // The computed value stands for the view, which reads the same fields.
    const seen = computed(() => {
      const labels: string[] = []
      walk(explorer.visibleNodes, (entry) => {
        labels.push(`${entry.label}${entry.loading ? '…' : ''}${entry.loaded ? '+' : ''}`)
      })
      return labels
    })
    const root = explorer.addRoot('c1')
    expect(seen.value).toEqual(['Server'])

    // An expand of the root and of a database below it.
    await explorer.expand(root)
    expect(seen.value).toEqual(['Server+', 'Sales', 'Other'])
    const sales = root.children![0]!
    await explorer.expand({ ...sales })
    expect(seen.value).toEqual(['Server+', 'Sales+', 'dbo', 'Other'])
    expect(isReactive(sales) && isShallow(sales)).toBe(true)

    // A refresh with a new answer.
    apiStub.listDatabases.mockResolvedValue([{ name: 'Archive' }])
    await explorer.refresh(root)
    expect(seen.value).toEqual(['Server+', 'Archive'])

    // A filter, and then a close of the connection.
    explorer.filter = 'arch'
    await afterTheFilterPause()
    expect(seen.value).toEqual(['Server+', 'Archive'])
    explorer.filter = 'nothing'
    await afterTheFilterPause()
    expect(seen.value).toEqual([])
    explorer.filter = ''
    await nextTick()
    explorer.removeRoot('c1')
    expect(seen.value).toEqual([])
  })

  it('keeps a leaf of the tree out of the reactivity', async () => {
    apiStub.listColumns.mockResolvedValue([
      { name: 'id', dataType: 'int', nullable: false, isPrimaryKey: true },
    ])
    const explorer = await readyStore()
    const columns = folderNode(
      'Columns',
      'columns',
      node({ key: 'c1/t', nodeType: 'table', table: 't' }),
    )
    await explorer.expand(columns)
    const leaf = columns.children![0]!
    expect(reactive(leaf)).toBe(leaf)
  })

  it('reads a root again after a copy with the key of a child took its place', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    // The copy has the key of the database, but the index names the node.
    root.children = [{ ...root.children![0]! }]
    await explorer.refresh(root)
    const fresh = root.children![0]!
    apiStub.listSchemas.mockResolvedValue([{ name: 'dbo' }])
    await explorer.expand({ ...fresh })
    expect(fresh.children?.map((child) => child.label)).toEqual(['dbo'])
  })

  it('reads a root again whose children are gone', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    root.children = undefined
    await explorer.refresh(root)
    expect(explorer.roots[0]?.children?.map((child) => child.label)).toEqual(['Sales'])
  })

  it('shows only the nodes that match the filter', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'Other' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    explorer.filter = 'sales'
    // The filter holds the text for a short pause, so the whole tree still
    // stands here.
    await afterTheFilterPause()
    expect(explorer.visibleNodes[0]?.children).toHaveLength(1)
  })

  it('waits for a pause before it matches the tree', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'Other' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)

    explorer.filter = 'sal'
    explorer.filter = 'sales'
    expect(explorer.visibleNodes[0]?.children).toHaveLength(2)

    await afterTheFilterPause()
    expect(explorer.visibleNodes[0]?.children).toHaveLength(1)
  })

  it('starts the pause again for a keystroke that follows the last one', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'Other' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)

    explorer.filter = 'sal'
    // The watcher runs between the two keystrokes, so the second one holds
    // back the pause that the first one started.
    await new Promise((resolve) => setTimeout(resolve, 0))
    explorer.filter = 'sales'
    await afterTheFilterPause()

    expect(explorer.visibleNodes[0]?.children).toHaveLength(1)
  })

  it('shows the whole tree as soon as the filter is empty', async () => {
    apiStub.listDatabases.mockResolvedValue([{ name: 'Sales' }, { name: 'Other' }])
    const explorer = await readyStore()
    const root = explorer.addRoot('c1')
    await explorer.expand(root)
    explorer.filter = 'sales'
    await afterTheFilterPause()

    explorer.filter = '  '
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(explorer.visibleNodes[0]?.children).toHaveLength(2)
  })
})
