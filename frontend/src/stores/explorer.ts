// src/stores/explorer.ts
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/tauri'
import { useConnectionStore } from './connection'
import { DbType } from '@/types/db'

// A self-contained interface for our tree nodes.
export interface ExplorerNode {
  key: string
  label: string
  icon?: string
  children?: ExplorerNode[]
  // Contains metadata about the node
  data: {
    type: 'database' | 'schema' | 'table' | 'column'
    [key: string]: any
  }
}

interface BackendDatabase { name: string }
interface BackendSchema { name: string }
interface BackendTable { name: string }

export const useExplorerStore = defineStore('explorer', () => {
  const nodes = ref<ExplorerNode[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)

  function clearExplorer() {
    nodes.value = []
    error.value = null
  }

  async function fetchDatabases() {
    const connectionStore = useConnectionStore()
    if (!connectionStore.isConnected) return

    loading.value = true
    error.value = null
    try {
      const result = await invoke<BackendDatabase[]>('list_databases')
      nodes.value = result.map((db: BackendDatabase) => ({
        key: db.name,
        label: db.name,
        icon: 'mdi-database',
        children: [], // Start with no children, expand on click
        data: { type: 'database', db: db.name },
      }))
    } catch (e: any) {
      error.value = e.toString()
      nodes.value = []
    } finally {
      loading.value = false
    }
  }

  async function expandNode(node: ExplorerNode) {
    const connectionStore = useConnectionStore()
    if (!connectionStore.isConnected) return

    // If children are already loaded, do nothing.
    if (node.children && node.children.length > 0) {
      return
    }

    loading.value = true
    error.value = null
    try {
      const { type, db, schema } = node.data

      if (type === 'database') {
        if (connectionStore.dbType === DbType.Mysql) {
          const tables = await invoke<BackendTable[]>('list_tables', { database: db })
          node.children = tables.map((t: BackendTable) => ({
            key: `${db}-${t.name}`,
            label: t.name,
            icon: 'mdi-table',
            data: { type: 'table', db, schema: db, table: t.name },
          }))
        } else {
          const schemas = await invoke<BackendSchema[]>('list_schemas', { database: db })
          node.children = schemas.map((s: BackendSchema) => ({
            key: `${db}-${s.name}`,
            label: s.name,
            icon: 'mdi-folder-outline',
            children: [],
            data: { type: 'schema', db, schema: s.name },
          }))
        }
      } else if (type === 'schema') {
        const tables = await invoke<BackendTable[]>('list_tables', { database: db, schemaName: schema })
        node.children = tables.map((t: BackendTable) => ({
          key: `${db}-${schema}-${t.name}`,
          label: t.name,
          icon: 'mdi-table',
          data: { type: 'table', db, schema, table: t.name },
        }))
      }
    } catch (e: any) {
      error.value = e.toString()
      // Optionally add an error node to provide feedback in the UI
      node.children = [{ key: `${node.key}-error`, label: 'Error', icon: 'mdi-alert-circle-outline', data: {type: 'column'} }]
    } finally {
      loading.value = false
    }
  }


  return {
    nodes,
    loading,
    error,
    clearExplorer,
    fetchDatabases,
    expandNode,
  }
})
