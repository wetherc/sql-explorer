import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/tauri'

interface Database {
  name: string;
}

interface Schema {
  name: string;
}

interface Table {
  TABLE_NAME: string;
}

interface Column {
  name: string;
  data_type: string;
}

export const useExplorerStore = defineStore('explorer', () => {
  const databases = ref<Database[]>([])
  const schemas = ref<Schema[]>([])
  const tables = ref<Table[]>([])
  const columns = ref<Column[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)

  async function fetchDatabases() {
    loading.value = true
    error.value = null
    try {
      const result = await invoke<Database[]>('list_databases')
      databases.value = result
    } catch (err) {
      error.value = err as string
    } finally {
      loading.value = false
    }
  }

  async function fetchSchemas() {
    loading.value = true
    error.value = null
    try {
      const result = await invoke<Schema[]>('list_schemas')
      schemas.value = result
    } catch (err) {
      error.value = err as string
    } finally {
      loading.value = false
    }
  }

  async function fetchTables(schemaName: string) {
    loading.value = true
    error.value = null
    try {
      const result = await invoke<Table[]>('list_tables', { schemaName })
      tables.value = result
    } catch (err) {
      error.value = err as string
    } finally {
      loading.value = false
    }
  }

  async function fetchColumns(schemaName: string, tableName: string) {
    loading.value = true
    error.value = null
    try {
      const result = await invoke<Column[]>('list_columns', { schemaName, tableName })
      columns.value = result
    } catch (err) {
      error.value = err as string
    } finally {
      loading.value = false
    }
  }

  return {
    databases,
    schemas,
    tables,
    columns,
    loading,
    error,
    fetchDatabases,
    fetchSchemas,
    fetchTables,
    fetchColumns,
  }
})
