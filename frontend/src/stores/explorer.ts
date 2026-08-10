import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api } from '@/lib/api'
import { useConnectionsStore } from './connections'
import { useUiStore } from './ui'
import { emptySchemaIndex, type SchemaIndex } from '@/lib/sql'
import { TableKind, type ColumnRef, type TableRef } from '@/types/api'

export type NodeKind = 'connection' | 'database' | 'schema' | 'table' | 'view' | 'column'

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
}

/** Selects the icon of a node. */
export function iconFor(kind: NodeKind, isKey = false): string {
  switch (kind) {
    case 'connection':
      return 'mdi-server'
    case 'database':
      return 'mdi-database'
    case 'schema':
      return 'mdi-folder-outline'
    case 'table':
      return 'mdi-table'
    case 'view':
      return 'mdi-table-eye'
    default:
      return isKey ? 'mdi-key-variant' : 'mdi-table-column'
  }
}

/** True when the node can hold children. */
export function isExpandable(node: ExplorerNode): boolean {
  return node.kind !== 'column'
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
    key: `${connectionId}/${database}/${schema ?? ''}/${table.name}`,
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
    const children = (node.children ?? []).map(keep).filter((child): child is ExplorerNode => child !== null)
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
    const seen = { databases: new Set<string>(), schemas: new Set<string>(), tables: new Set<string>() }
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

    if (node.kind === 'database') {
      const database = node.database ?? node.label
      if (supportsSchemas) {
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
      const tables = await api.listTables(node.connectionId, database, null)
      return tables.map((table) => tableNode(table, node.connectionId, database, undefined))
    }

    if (node.kind === 'schema') {
      const database = node.database ?? ''
      const tables = await api.listTables(node.connectionId, database, node.schema ?? null)
      return tables.map((table) => tableNode(table, node.connectionId, database, node.schema))
    }

    // A table or a view holds its columns.
    const columns = await api.listColumns(
      node.connectionId,
      node.database ?? '',
      node.schema ?? null,
      node.table ?? node.label,
    )
    return columns.map((column) => columnNode(column, node))
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
