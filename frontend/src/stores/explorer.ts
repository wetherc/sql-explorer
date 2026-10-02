import { defineStore } from 'pinia'
import { computed, markRaw, ref, shallowReactive, shallowRef, watch, type ComputedRef } from 'vue'
import { api } from '@/lib/api'
import { useConnectionsStore } from './connections'
import { useSettingsStore } from './settings'
import { useUiStore } from './ui'
import { emptySchemaIndex, type SchemaIndex } from '@/lib/sql'
import {
  Dialect,
  TableKind,
  type ColumnRef,
  type SchemaSnapshot,
  type ConstraintRef,
  type IndexRef,
  type DriverCapabilities,
  type TableRef,
} from '@/types/api'

/** The pause between the last keystroke in the filter and the match. */
export const FILTER_DELAY_MS = 200

export type NodeKind =
  | 'connection'
  | 'database'
  | 'schema'
  | 'folder'
  | 'table'
  | 'view'
  | 'column'
  | 'routine'
  | 'index'
  | 'constraint'
  | 'partition'

/** What a folder node holds, which decides the call that fills it. */
export type FolderKind =
  | 'tables'
  | 'views'
  | 'procedures'
  | 'functions'
  | 'columns'
  | 'indexes'
  | 'constraints'
  | 'partitions'

/** The kinds that hold no children of their own. */
const LEAF_KINDS: NodeKind[] = ['column', 'routine', 'index', 'constraint', 'partition']

export interface ExplorerNode {
  key: string
  label: string
  kind: NodeKind
  icon: string
  /** Extra text the tree shows after the label, such as a column type. */
  hint?: string
  /** Missing for a node that never expands, such as a column. */
  children?: ExplorerNode[]
  loading: boolean
  loaded: boolean
  connectionId: string
  database?: string
  schema?: string
  table?: string
  /** Set on a folder node, and it names the list the folder holds. */
  folder?: FolderKind
}

/** Selects the icon of a node. */
export function iconFor(kind: NodeKind, isKey = false): string {
  switch (kind) {
    case 'connection':
      return 'mdi-server'
    case 'database':
      return 'mdi-database'
    case 'schema':
    case 'folder':
      return 'mdi-folder-outline'
    case 'table':
      return 'mdi-table'
    case 'view':
      return 'mdi-table-eye'
    case 'routine':
      return 'mdi-function-variant'
    case 'index':
      return 'mdi-sort-alphabetical-variant'
    case 'constraint':
      return 'mdi-key-chain'
    case 'partition':
      return 'mdi-file-tree-outline'
    default:
      return isKey ? 'mdi-key-variant' : 'mdi-table-column'
  }
}

/** True when the node can hold children. */
export function isExpandable(node: ExplorerNode): boolean {
  return !LEAF_KINDS.includes(node.kind)
}

/** Builds the node of one relation. */
export function tableNode(
  table: TableRef,
  connectionId: string,
  database: string,
  schema: string | undefined,
): ExplorerNode {
  const kind: NodeKind = table.kind === TableKind.View ? 'view' : 'table'
  return {
    // The kind is part of the key, because a table and a view of one schema
    // can carry the same name.
    key: `${connectionId}/${database}/${schema ?? ''}/${kind}/${table.name}`,
    label: table.name,
    kind,
    icon: iconFor(kind),
    children: [],
    loading: false,
    loaded: false,
    connectionId,
    database,
    schema,
    table: table.name,
  }
}

/** Builds the node of one column. */
export function columnNode(column: ColumnRef, parent: ExplorerNode): ExplorerNode {
  return {
    key: `${parent.key}/${column.name}`,
    label: column.name,
    kind: 'column',
    icon: iconFor('column', column.isPrimaryKey),
    hint: `${column.dataType}${column.nullable ? '' : ' not null'}`,
    loading: false,
    loaded: true,
    connectionId: parent.connectionId,
    database: parent.database,
    schema: parent.schema,
    table: parent.table,
  }
}

/** Builds a folder node below a schema, a database or a relation. */
export function folderNode(label: string, folder: FolderKind, parent: ExplorerNode): ExplorerNode {
  return {
    key: `${parent.key}/${folder}`,
    label,
    kind: 'folder',
    icon: iconFor('folder'),
    children: [],
    loading: false,
    loaded: false,
    connectionId: parent.connectionId,
    database: parent.database,
    schema: parent.schema,
    table: parent.table,
    folder,
  }
}

/** Builds a node that holds no children, below a folder. */
export function leafNode(
  label: string,
  kind: NodeKind,
  parent: ExplorerNode,
  hint?: string,
): ExplorerNode {
  return {
    key: `${parent.key}/${label}`,
    label,
    kind,
    icon: iconFor(kind),
    hint,
    loading: false,
    loaded: true,
    connectionId: parent.connectionId,
    database: parent.database,
    schema: parent.schema,
    table: parent.table,
  }
}

/**
 * Gives each node of one list a key of its own. A key holds the name of the
 * node, and two routines of PostgreSQL can share a name when their
 * arguments differ. A repeated key gets the number of its repeat, so the
 * view gets no two rows with one key.
 */
export function withUniqueKeys(nodes: ExplorerNode[]): ExplorerNode[] {
  const counts = new Map<string, number>()
  return nodes.map((node) => {
    const count = (counts.get(node.key) ?? 0) + 1
    counts.set(node.key, count)
    return count === 1 ? node : { ...node, key: `${node.key}#${count}` }
  })
}

/** Names one constraint for the tree: its kind, and its columns. */
export function constraintHint(constraint: ConstraintRef): string {
  const words: Record<ConstraintRef['kind'], string> = {
    primaryKey: 'primary key',
    foreignKey: 'foreign key',
    unique: 'unique',
    check: 'check',
    exclusion: 'exclusion',
    trigger: 'trigger',
    notNull: 'not null',
    default: 'default',
  }
  const parts = [words[constraint.kind]]
  if (constraint.columns.length > 0) {
    parts.push(constraint.columns.join(', '))
  }
  if (constraint.detail) {
    parts.push(constraint.detail)
  }
  return parts.join(' · ')
}

/** Names the key columns of an index, and then its `INCLUDE` columns. */
export function indexColumns(index: IndexRef): string {
  const key = index.columns.join(', ')
  return index.included.length > 0 ? `${key} include (${index.included.join(', ')})` : key
}

/**
 * Keeps the nodes whose label holds the filter text, and keeps a parent
 * whose child matches, so that the path to a match stays visible.
 *
 * A node that matches keeps all of its children. The user who opens a
 * matching node sees what it holds, and not the note of an empty branch.
 */
export function filterNodes(nodes: ExplorerNode[], filter: string): ExplorerNode[] {
  const needle = filter.trim().toLowerCase()
  if (needle === '') {
    return nodes
  }
  const keep = (node: ExplorerNode): ExplorerNode | null => {
    const children = (node.children ?? [])
      .map(keep)
      .filter((child): child is ExplorerNode => child !== null)
    const matches = node.label.toLowerCase().includes(needle)
    if (!matches && children.length === 0) {
      return null
    }
    if (!node.children) {
      return { ...node, children: undefined }
    }
    return { ...node, children: matches ? node.children : children }
  }
  return nodes.map(keep).filter((node): node is ExplorerNode => node !== null)
}

/** Walks the tree and calls the visitor for every node. */
export function walk(nodes: ExplorerNode[], visit: (node: ExplorerNode) => void): void {
  for (const node of nodes) {
    visit(node)
    if (node.children) {
      walk(node.children, visit)
    }
  }
}

export const useExplorerStore = defineStore('explorer', () => {
  const connections = useConnectionsStore()
  const settings = useSettingsStore()
  const ui = useUiStore()

  /**
   * The roots of the tree, one for each open connection.
   *
   * A schema can have tens of thousands of relations, so the tree is not
   * deep reactive. The list of roots sits in a shallow reference, and each
   * node that can expand is a shallow reactive object. A read writes a new
   * list into `children`, and the view sees that write. A leaf never
   * changes, so it is marked raw. The store changes the list through
   * addRoot, removeRoot and clear, because these keep `nodeIndex` in step.
   */
  const roots = shallowRef<ExplorerNode[]>([])
  /** Each node of the tree, by its key, so a look-up walks no tree. */
  const nodeIndex = new Map<string, ExplorerNode>()
  /** The nodes whose read runs, so the reading flag walks no tree. */
  const loadingNodes = new Set<ExplorerNode>()
  const filter = ref('')
  const loading = ref(false)
  /**
   * The number of the last read of the children of each node, by the key of
   * the node. A refresh raises the number, so the answer of a read that the
   * refresh passed can be told apart and dropped.
   */
  const loadGeneration = new Map<string, number>()

  /**
   * The filter text that the tree is matched against. It follows the field
   * after a short pause, because a match builds a copy of every node that
   * survives it, and a keystroke would build that copy again.
   */
  const appliedFilter = ref('')
  let filterTimer: ReturnType<typeof setTimeout> | null = null

  function applyFilter(value: string): void {
    if (filterTimer !== null) {
      clearTimeout(filterTimer)
      filterTimer = null
    }
    // An empty filter shows the whole tree at once, because the user who
    // clears the field waits for nothing.
    if (value.trim() === '') {
      appliedFilter.value = ''
      return
    }
    filterTimer = setTimeout(() => {
      filterTimer = null
      appliedFilter.value = value
    }, FILTER_DELAY_MS)
  }

  watch(filter, applyFilter)

  const visibleNodes = computed(() => filterNodes(roots.value, appliedFilter.value))

  /**
   * The whole schema of one database of one connection, keyed by the two
   * names. The editor reads these names, so a relation the user never opened
   * in the tree is still offered.
   *
   * A snapshot holds up to tens of thousands of column records and never
   * changes after it arrives, so the record sits in a shallow reference and
   * each snapshot is marked raw. A deep proxy over that many objects would
   * cost memory and tracking for nothing. Every write replaces the whole
   * record, which is what a shallow reference reacts to.
   */
  const snapshots = shallowRef<Record<string, SchemaSnapshot>>({})

  /** The bounds the settings put on a read of a schema. */
  function snapshotOptions(): { maxColumns: number; ownConnection: boolean } {
    return {
      maxColumns: settings.settings.schemaSnapshotColumns,
      ownConnection: settings.settings.schemaSnapshotOwnConnection,
    }
  }

  /**
   * The count of the drops of the snapshots of each connection, and of the
   * drops of all of them. A read that started before a drop gives an answer
   * for a connection that the user closed, so the store drops that answer.
   */
  const forgetCounts = new Map<string, number>()
  let clearCount = 0

  function forgetStamp(connectionId: string): string {
    return `${clearCount}/${forgetCounts.get(connectionId) ?? 0}`
  }

  /** The key one snapshot lives under. */
  function snapshotKey(connectionId: string, database: string): string {
    return `${connectionId}/${database}`
  }

  /** The reads of a snapshot that run, by the key of the snapshot. */
  const pendingSnapshots = new Map<string, Promise<SchemaSnapshot | null>>()

  /**
   * Reads the schema of one database and keeps it. A read that is already
   * held is not made again, so a change of the current database costs one
   * read for each database and no more. A second caller during a read gets
   * the promise of that read, so one database costs one call to the backend.
   * A forced read starts a new call, and the answer of the older call then
   * does not go into the store.
   */
  function readSnapshot(
    connectionId: string,
    database: string,
    options: { maxColumns: number; ownConnection: boolean },
    force = false,
  ): Promise<SchemaSnapshot | null> {
    const key = snapshotKey(connectionId, database)
    const held = snapshots.value[key]
    if (!force && held) {
      return Promise.resolve(held)
    }
    const pending = pendingSnapshots.get(key)
    if (!force && pending) {
      return pending
    }
    const isLast = () => pendingSnapshots.get(key) === read
    const read = fetchSnapshot(connectionId, database, options, isLast).finally(() => {
      if (isLast()) {
        pendingSnapshots.delete(key)
      }
    })
    pendingSnapshots.set(key, read)
    return read
  }

  /**
   * Asks the backend for the schema of one database. The answer goes into
   * the store only while `isLast` holds, because a forced read can start
   * after this one.
   */
  async function fetchSnapshot(
    connectionId: string,
    database: string,
    options: { maxColumns: number; ownConnection: boolean },
    isLast: () => boolean,
  ): Promise<SchemaSnapshot | null> {
    const key = snapshotKey(connectionId, database)
    const stamp = forgetStamp(connectionId)
    try {
      const snapshot = await api.schemaSnapshot({
        connectionId,
        database,
        maxColumns: options.maxColumns,
        ownConnection: options.ownConnection,
      })
      if (!Array.isArray(snapshot?.relations) || forgetStamp(connectionId) !== stamp) {
        // An answer of another shape is left out, so that the names of the
        // editor stay a list this store can read. An answer for a connection
        // that closed during the read is left out too.
        return null
      }
      if (!isLast()) {
        return snapshot
      }
      snapshots.value = { ...snapshots.value, [key]: markRaw(snapshot) }
      if (!snapshot.complete) {
        ui.warn(
          `The schema of ${database} is larger than the limit of ${options.maxColumns} columns, ` +
            'so the editor offers a part of it. Raise the limit in the settings.',
        )
      }
      return snapshot
    } catch (error) {
      // A schema that cannot be read leaves the editor with the names of the
      // tree, so the failure is reported and nothing else stops.
      ui.reportError(error)
      return null
    }
  }

  /** Drops the snapshots of one connection. */
  function forgetSnapshots(connectionId: string): void {
    forgetCounts.set(connectionId, (forgetCounts.get(connectionId) ?? 0) + 1)
    schemaIndexes.delete(connectionId)
    for (const key of [...pendingSnapshots.keys()]) {
      if (key.startsWith(`${connectionId}/`)) {
        pendingSnapshots.delete(key)
      }
    }
    const kept: Record<string, SchemaSnapshot> = {}
    for (const [key, snapshot] of Object.entries(snapshots.value)) {
      if (!key.startsWith(`${connectionId}/`)) {
        kept[key] = snapshot
      }
    }
    snapshots.value = kept
  }

  /** An empty part of the index, with the names it saw. */
  function emptyPart(): {
    index: SchemaIndex
    seen: Record<'databases' | 'schemas' | 'tables', Set<string>>
  } {
    return {
      index: emptySchemaIndex(),
      seen: { databases: new Set(), schemas: new Set(), tables: new Set() },
    }
  }

  /**
   * The part of the index that the snapshots hold, with the names it saw, by
   * the identifier of the connection. It lives apart from the whole index so
   * that a change of the tree leaves this part cached, because the snapshots
   * hold most of the names.
   */
  const snapshotParts = computed(() => {
    const parts = new Map<string, ReturnType<typeof emptyPart>>()
    for (const [key, snapshot] of Object.entries(snapshots.value)) {
      const connectionId = key.slice(0, key.indexOf('/'))
      let part = parts.get(connectionId)
      if (!part) {
        part = emptyPart()
        parts.set(connectionId, part)
      }
      const { index, seen } = part
      if (!seen.databases.has(snapshot.database)) {
        seen.databases.add(snapshot.database)
        index.databases.push(snapshot.database)
      }
      for (const relation of snapshot.relations) {
        if (relation.schema && !seen.schemas.has(relation.schema)) {
          seen.schemas.add(relation.schema)
          index.schemas.push(relation.schema)
        }
        const qualifier = [snapshot.database, relation.schema].filter(Boolean).join('.')
        const identity = `${qualifier}/${relation.name}`
        if (seen.tables.has(identity)) {
          continue
        }
        seen.tables.add(identity)
        index.tables.push({ name: relation.name, qualifier })
        for (const column of relation.columns) {
          index.columns.push({
            name: column.name,
            table: relation.name,
            qualifier,
            dataType: column.dataType,
          })
        }
      }
    }
    return parts
  })

  /** Builds the names that the editor offers for one connection. */
  function buildSchemaIndex(connectionId: string): SchemaIndex {
    // The snapshots come first, because they hold the whole database. The
    // copies hold references to the entries of the cached part, so a change
    // of the tree pays for the copies and the walk, not for a rebuild of
    // the snapshot part.
    const base = snapshotParts.value.get(connectionId) ?? emptyPart()
    const index: SchemaIndex = {
      databases: [...base.index.databases],
      schemas: [...base.index.schemas],
      tables: [...base.index.tables],
      columns: [...base.index.columns],
    }
    const seen = {
      databases: new Set(base.seen.databases),
      schemas: new Set(base.seen.schemas),
      tables: new Set(base.seen.tables),
    }

    // The tree adds what the user has opened and the snapshots do not hold.
    // The walk reads the set of the cached part and mutates the copies only.
    const fromSnapshot = base.seen.tables
    const own = roots.value.filter((root) => root.connectionId === connectionId)
    walk(own, (node) => {
      const qualifier = [node.database, node.schema].filter(Boolean).join('.')
      const identity = `${qualifier}/${node.table ?? node.label}`
      if (node.kind === 'database' && !seen.databases.has(node.label)) {
        seen.databases.add(node.label)
        index.databases.push(node.label)
      } else if (node.kind === 'schema' && !seen.schemas.has(node.label)) {
        seen.schemas.add(node.label)
        index.schemas.push(node.label)
      } else if (node.kind === 'table' || node.kind === 'view') {
        if (!seen.tables.has(identity)) {
          seen.tables.add(identity)
          index.tables.push({ name: node.label, qualifier })
        }
      } else if (node.kind === 'column' && !fromSnapshot.has(identity)) {
        // A relation that a snapshot already holds keeps the columns of the
        // snapshot, so no name appears twice.
        index.columns.push({
          name: node.label,
          table: node.table ?? '',
          qualifier,
          dataType: node.hint ?? '',
        })
      }
    })
    return index
  }

  /** The cached index of each connection that an editor asked for. */
  const schemaIndexes = new Map<string, ComputedRef<SchemaIndex>>()
  const noNames = emptySchemaIndex()

  /**
   * The names that the editor offers for one connection. A tab sends its
   * statement to one connection, so the names of the other connections
   * stay out. A tab with no connection gets no names.
   */
  function schemaIndexFor(connectionId: string | null): SchemaIndex {
    if (connectionId === null) {
      return noNames
    }
    let index = schemaIndexes.get(connectionId)
    if (!index) {
      index = computed(() => buildSchemaIndex(connectionId))
      schemaIndexes.set(connectionId, index)
    }
    return index.value
  }

  /** Builds the root node of one connection. */
  function rootFor(connectionId: string): ExplorerNode {
    const connection = connections.byId(connectionId)
    return {
      key: connectionId,
      label: connection?.name ?? connectionId,
      kind: 'connection',
      icon: iconFor('connection'),
      children: [],
      loading: false,
      loaded: false,
      connectionId,
    }
  }

  /** Gives a new node the reactivity that the tree needs. */
  function adopt(node: ExplorerNode): ExplorerNode {
    return isExpandable(node) ? shallowReactive(node) : markRaw(node)
  }

  /** True when the node is part of the tree, and not a copy or a node that went. */
  function inTree(node: ExplorerNode): boolean {
    return nodeIndex.get(node.key) === node
  }

  function indexNodes(nodes: ExplorerNode[]): void {
    walk(nodes, (node) => nodeIndex.set(node.key, node))
  }

  function unindexNodes(nodes: ExplorerNode[]): void {
    walk(nodes, (node) => {
      if (inTree(node)) {
        nodeIndex.delete(node.key)
      }
    })
  }

  /** Writes the children of a node, and keeps the index in step. */
  function setChildren(node: ExplorerNode, children: ExplorerNode[]): void {
    const indexed = inTree(node)
    if (indexed) {
      unindexNodes(node.children ?? [])
    }
    node.children = children
    if (indexed) {
      indexNodes(children)
    }
  }

  /** Adds a root for a connection that has just opened. */
  function addRoot(connectionId: string): ExplorerNode {
    const existing = roots.value.find((node) => node.key === connectionId)
    if (existing) {
      return existing
    }
    const node = adopt(rootFor(connectionId))
    roots.value = [...roots.value, node]
    indexNodes([node])
    return node
  }

  function removeRoot(connectionId: string): void {
    unindexNodes(roots.value.filter((node) => node.key === connectionId))
    roots.value = roots.value.filter((node) => node.key !== connectionId)
    forgetSnapshots(connectionId)
    forgetRelations({ connectionId })
  }

  function clear(): void {
    roots.value = []
    nodeIndex.clear()
    loadingNodes.clear()
    relationReads.clear()
    filter.value = ''
    // The watch of the field runs later, and the tree is empty now.
    applyFilter('')
    snapshots.value = {}
    pendingSnapshots.clear()
    clearCount += 1
    schemaIndexes.clear()
  }

  /** Finds the node with the given key in the tree itself. */
  function nodeByKey(key: string): ExplorerNode | null {
    return nodeIndex.get(key) ?? null
  }

  /**
   * Reads the children of a node from the server.
   *
   * The filter of the tree hands out copies, so the work runs on the node
   * with the same key inside the tree itself. A write into a copy would
   * never reach the view.
   */
  async function expand(given: ExplorerNode): Promise<void> {
    const node = nodeByKey(given.key) ?? given
    if (!isExpandable(node) || node.loading || node.loaded) {
      return
    }
    await load(node)
  }

  /**
   * Reads the children of a node again, also while a read of the same node
   * runs. The newer read wins, and the answer of the older one is dropped.
   *
   * The read gives new child nodes with no children of their own. `open`
   * holds the keys of the branches that the tree shows open, and each such
   * branch below the node is read again, so it does not stand open and
   * empty.
   */
  async function refresh(
    given: ExplorerNode,
    open: ReadonlySet<string> = new Set(),
  ): Promise<void> {
    const node = nodeByKey(given.key) ?? given
    if (!isExpandable(node)) {
      return
    }
    node.loaded = false
    setChildren(node, [])
    forgetRelations(node)
    // The user asks for the objects of the server again, so the schema that
    // the editor offers is read again too. A database that is shut gets its
    // schema when the user next opens it.
    if (node.kind === 'connection') {
      forgetSnapshots(node.connectionId)
    }
    await reopen(await load(node, true), open)
  }

  /** Reads each open branch among the children that a read just gave. */
  async function reopen(children: ExplorerNode[], open: ReadonlySet<string>): Promise<void> {
    for (const child of children) {
      if (open.has(child.key) && isExpandable(child)) {
        await reopen(await load(child, true), open)
      }
    }
  }

  /**
   * Reads the children of one node and writes them into it.
   *
   * Each read carries a number of its own. An answer whose number is no
   * longer the last one of the node is dropped, because a refresh has since
   * started a newer read of the same node.
   *
   * Returns the children that the read wrote, and an empty list for a read
   * that failed or that a newer read passed. A read with `fresh` also reads
   * the schema of a database again.
   */
  async function load(node: ExplorerNode, fresh = false): Promise<ExplorerNode[]> {
    const generation = (loadGeneration.get(node.key) ?? 0) + 1
    loadGeneration.set(node.key, generation)
    const isLast = () => loadGeneration.get(node.key) === generation
    node.loading = true
    loadingNodes.add(node)
    loading.value = true
    try {
      const children = (await childrenOf(node)).map(adopt)
      if (!isLast()) {
        return []
      }
      setChildren(node, children)
      node.loaded = true
      if (node.kind === 'database') {
        // The user has shown interest in this database, so the whole schema
        // is read for the completions of the editor. The read runs on its own
        // and the tree does not wait for it.
        void readSnapshot(node.connectionId, node.database ?? node.label, snapshotOptions(), fresh)
      }
      return children
    } catch (error) {
      if (isLast()) {
        ui.reportError(error)
        setChildren(node, [])
        node.loaded = false
      }
      return []
    } finally {
      if (isLast()) {
        node.loading = false
        loadingNodes.delete(node)
      }
      // A node that left the tree during its read does not count.
      loading.value = [...loadingNodes].some(inTree)
    }
  }

  /** Asks the backend for the level below the given node. */
  async function childrenOf(node: ExplorerNode): Promise<ExplorerNode[]> {
    const info = connections.active[node.connectionId]
    const supportsSchemas = info?.capabilities.supportsSchemas ?? false

    if (node.kind === 'connection') {
      const databases = await api.listDatabases(node.connectionId)
      return databases.map((database) => ({
        key: `${node.connectionId}/${database.name}`,
        label: database.name,
        kind: 'database' as const,
        icon: iconFor('database'),
        children: [],
        loading: false,
        loaded: false,
        connectionId: node.connectionId,
        database: database.name,
      }))
    }

    if (node.kind === 'database' && supportsSchemas) {
      const database = node.database ?? node.label
      const schemas = await api.listSchemas(node.connectionId, database)
      // A SQLite file with no temporary and no attached database has the
      // schema main alone. The tree then puts the folders below the
      // database, and a read without a schema reads main.
      if (info?.dialect === Dialect.Sqlite && schemas.length === 1 && schemas[0]?.name === 'main') {
        return schemaFolders(node, info.capabilities)
      }
      return schemas.map((schema) => ({
        key: `${node.connectionId}/${database}/${schema.name}`,
        label: schema.name,
        kind: 'schema' as const,
        icon: iconFor('schema'),
        children: [],
        loading: false,
        loaded: false,
        connectionId: node.connectionId,
        database,
        schema: schema.name,
      }))
    }

    // A schema, and a database of an engine without schemas, hold folders.
    if (node.kind === 'database' || node.kind === 'schema') {
      return schemaFolders(node, info?.capabilities)
    }

    if (node.kind === 'table' || node.kind === 'view') {
      return relationFolders(node, info?.capabilities)
    }

    return withUniqueKeys(await folderChildren(node))
  }

  /** The folders below a schema, or below a database without schemas. */
  function schemaFolders(
    node: ExplorerNode,
    capabilities: DriverCapabilities | undefined,
  ): ExplorerNode[] {
    const folders = [folderNode('Tables', 'tables', node), folderNode('Views', 'views', node)]
    if (capabilities?.supportsRoutines) {
      folders.push(folderNode('Procedures', 'procedures', node))
      folders.push(folderNode('Functions', 'functions', node))
    }
    return folders
  }

  /**
   * The folders below a relation. A view holds columns alone, because an
   * index and a constraint belong to a table.
   */
  function relationFolders(
    node: ExplorerNode,
    capabilities: DriverCapabilities | undefined,
  ): ExplorerNode[] {
    const folders = [folderNode('Columns', 'columns', node)]
    if (node.kind === 'view') {
      return folders
    }
    if (capabilities?.supportsIndexes) {
      folders.push(folderNode('Indexes', 'indexes', node))
    }
    if (capabilities?.supportsConstraints) {
      folders.push(folderNode('Keys', 'constraints', node))
    }
    if (capabilities?.supportsPartitions) {
      folders.push(folderNode('Partitions', 'partitions', node))
    }
    return folders
  }

  /**
   * The reads of the relations of one schema, by the place they read. The
   * Tables folder and the Views folder show two parts of one list, so the
   * two folders share one call to the backend. An entry goes away when both
   * folders have read it, when its read fails, and when the user refreshes
   * or closes its place.
   */
  const relationReads = new Map<
    string,
    {
      connectionId: string
      database: string
      schema: string | null
      list: Promise<TableRef[]>
      readers: Set<FolderKind>
    }
  >()

  /** Reads the relations of one schema for one of its two folders. */
  function relationsOf(node: ExplorerNode, database: string, schema: string | null) {
    const key = JSON.stringify([node.connectionId, database, schema])
    let read = relationReads.get(key)
    if (!read) {
      const list = api.listTables(node.connectionId, database, schema)
      const entry = {
        connectionId: node.connectionId,
        database,
        schema,
        list,
        readers: new Set<FolderKind>(),
      }
      list.catch(() => {
        if (relationReads.get(key) === entry) {
          relationReads.delete(key)
        }
      })
      relationReads.set(key, entry)
      read = entry
    }
    read.readers.add(node.folder!)
    if (read.readers.size === 2) {
      relationReads.delete(key)
    }
    return read.list
  }

  /** Drops the shared reads of relations at the place of a node and below it. */
  function forgetRelations(place: { connectionId: string; database?: string; schema?: string }) {
    for (const [key, read] of relationReads) {
      if (
        read.connectionId === place.connectionId &&
        (place.database === undefined || read.database === place.database) &&
        (place.schema === undefined || read.schema === place.schema)
      ) {
        relationReads.delete(key)
      }
    }
  }

  /** Reads the list that one folder holds. */
  async function folderChildren(node: ExplorerNode): Promise<ExplorerNode[]> {
    const connectionId = node.connectionId
    const database = node.database ?? ''
    const schema = node.schema ?? null
    const table = node.table ?? ''

    switch (node.folder) {
      case 'tables':
      case 'views': {
        const wanted = node.folder === 'views' ? TableKind.View : TableKind.Table
        const tables = await relationsOf(node, database, schema)
        return tables
          .filter((entry) => entry.kind === wanted)
          .map((entry) => tableNode(entry, connectionId, database, node.schema))
      }
      case 'procedures':
      case 'functions': {
        const wanted = node.folder === 'procedures' ? 'procedure' : 'function'
        const routines = await api.listRoutines(connectionId, database, schema)
        return routines
          .filter((routine) => routine.kind === wanted)
          .map((routine) => leafNode(routine.name, 'routine', node))
      }
      case 'indexes': {
        const indexes = await api.listIndexes(connectionId, database, schema, table)
        return indexes.map((index) =>
          leafNode(
            index.name,
            'index',
            node,
            [indexColumns(index), index.primary ? 'primary key' : index.unique ? 'unique' : '']
              .filter(Boolean)
              .join(' · '),
          ),
        )
      }
      case 'constraints': {
        const constraints = await api.listConstraints(connectionId, database, schema, table)
        return constraints.map((constraint) =>
          leafNode(constraint.name, 'constraint', node, constraintHint(constraint)),
        )
      }
      case 'partitions': {
        const list = await api.listPartitions(connectionId, database, schema, table)
        node.hint = list.truncated ? `first ${list.partitions.length}` : undefined
        return list.partitions.map((partition) => leafNode(partition.values, 'partition', node))
      }
      default: {
        const columns = await api.listColumns(connectionId, database, schema, table)
        return columns.map((column) => columnNode(column, node))
      }
    }
  }

  return {
    roots,
    filter,
    loading,
    visibleNodes,
    schemaIndexFor,
    snapshots,
    snapshotOptions,
    readSnapshot,
    forgetSnapshots,
    addRoot,
    removeRoot,
    clear,
    expand,
    refresh,
  }
})
