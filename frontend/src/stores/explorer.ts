import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api } from '@/lib/api'
import { useConnectionsStore } from './connections'
import { useUiStore } from './ui'
import { emptySchemaIndex, type SchemaIndex } from '@/lib/sql'
import {
  TableKind,
  type ColumnRef,
  type ConstraintRef,
  type DriverCapabilities,
  type TableRef,
} from '@/types/api'

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

/** Names one constraint for the tree: its kind, and its columns. */
export function constraintHint(constraint: ConstraintRef): string {
  const words: Record<ConstraintRef['kind'], string> = {
    primaryKey: 'primary key',
    foreignKey: 'foreign key',
    unique: 'unique',
    check: 'check',
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

/**
 * Keeps the nodes whose label holds the filter text, and keeps a parent
 * whose child survives, so that the path to a match stays visible.
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
    return { ...node, children: node.children ? children : undefined }
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
  const ui = useUiStore()

  /** The roots of the tree, one for each open connection. */
  const roots = ref<ExplorerNode[]>([])
  const filter = ref('')
  const loading = ref(false)

  const visibleNodes = computed(() => filterNodes(roots.value, filter.value))

  /** The names the editor offers as completions. */
  const schemaIndex = computed<SchemaIndex>(() => {
    const index = emptySchemaIndex()
    const seen = {
      databases: new Set<string>(),
      schemas: new Set<string>(),
      tables: new Set<string>(),
    }
    walk(roots.value, (node) => {
      if (node.kind === 'database' && !seen.databases.has(node.label)) {
        seen.databases.add(node.label)
        index.databases.push(node.label)
      } else if (node.kind === 'schema' && !seen.schemas.has(node.label)) {
        seen.schemas.add(node.label)
        index.schemas.push(node.label)
      } else if ((node.kind === 'table' || node.kind === 'view') && !seen.tables.has(node.key)) {
        seen.tables.add(node.key)
        index.tables.push({
          name: node.label,
          qualifier: [node.database, node.schema].filter(Boolean).join('.'),
        })
      } else if (node.kind === 'column') {
        index.columns.push({
          name: node.label,
          table: node.table ?? '',
          dataType: node.hint ?? '',
        })
      }
    })
    return index
  })

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

  /** Adds a root for a connection that has just opened. */
  function addRoot(connectionId: string): ExplorerNode {
    const existing = roots.value.find((node) => node.key === connectionId)
    if (existing) {
      return existing
    }
    const node = rootFor(connectionId)
    roots.value = [...roots.value, node]
    return node
  }

  function removeRoot(connectionId: string): void {
    roots.value = roots.value.filter((node) => node.key !== connectionId)
  }

  function clear(): void {
    roots.value = []
    filter.value = ''
  }

  /** Reads the children of a node from the server. */
  async function expand(node: ExplorerNode): Promise<void> {
    if (!isExpandable(node) || node.loading) {
      return
    }
    if (node.loaded) {
      return
    }
    node.loading = true
    loading.value = true
    try {
      node.children = await childrenOf(node)
      node.loaded = true
    } catch (error) {
      ui.reportError(error)
      node.children = []
      node.loaded = false
    } finally {
      node.loading = false
      loading.value = roots.value.some((root) => hasLoadingNode(root))
    }
  }

  /** Reads the children of a node again. */
  async function refresh(node: ExplorerNode): Promise<void> {
    node.loaded = false
    node.children = []
    await expand(node)
  }

  function hasLoadingNode(node: ExplorerNode): boolean {
    if (node.loading) {
      return true
    }
    return (node.children ?? []).some(hasLoadingNode)
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

    return folderChildren(node)
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
        const tables = await api.listTables(connectionId, database, schema)
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
            [index.columns.join(', '), index.primary ? 'primary key' : index.unique ? 'unique' : '']
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
        const partitions = await api.listPartitions(connectionId, database, schema, table)
        return partitions.map((partition) => leafNode(partition.values, 'partition', node))
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
    schemaIndex,
    addRoot,
    removeRoot,
    clear,
    expand,
    refresh,
  }
})
