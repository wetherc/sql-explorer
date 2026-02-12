import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/tauri'

interface Database {
  name: string;
}

interface Schema {
  name: string;
  database_name: string;
}

interface Table {
  name: string;
  database_name: string;
  schema_name: string;
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

  const nodes = computed<TreeNode[]>(() => {
    return databases.value.map(db => ({
      key: db.name,
      label: db.name,
      icon: 'pi pi-fw pi-database',
      data: { type: 'database', db: db.name },
      children: schemas.value.filter(schema => schema.database_name === db.name).map(schema => ({
        key: `${db.name}-${schema.name}`,
        label: schema.name,
        icon: 'pi pi-fw pi-folder',
        data: { type: 'schema', db: db.name, schema: schema.name },
        children: tables.value.filter(table => table.database_name === db.name && table.schema_name === schema.name).map(table => ({
          key: `${db.name}-${schema.name}-${table.name}`,
          label: table.name,
          icon: 'pi pi-fw pi-table',
          data: { type: 'table', db: db.name, schema: schema.name, table: table.name }
        }))
      }))
    }))
  })

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
