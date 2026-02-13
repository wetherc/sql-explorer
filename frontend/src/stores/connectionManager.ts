// frontend/src/stores/connectionManager.ts
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/tauri'
import { v4 as uuidv4 } from 'uuid'

export interface SavedConnection {
  id: string
  name: string
  dbType: 'mssql' | 'mysql' | 'postgres'
  host?: string
  port?: number
  user?: string
  database?: string
  password?: string
}

export const useConnectionManagerStore = defineStore('connectionManager', () => {
  const connections = ref<SavedConnection[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)
  const showConnectionForm = ref(false)
  const editingConnection = ref<SavedConnection | null>(null)

  async function fetchConnections() {
    loading.value = true
    error.value = null
    try {
      connections.value = await invoke('get_connections')
    } catch (e: any) {
      error.value = e.toString()
    } finally {
      loading.value = false
    }
  }

  async function saveConnection(connection: SavedConnection) {
    loading.value = true
    error.value = null
    try {
      if (!connection.id) {
        connection.id = uuidv4()
      }
      await invoke('save_connection', { connection })
      await fetchConnections() // Refresh the list
      showConnectionForm.value = false
    } catch (e: any) {
      error.value = e.toString()
    } finally {
      loading.value = false
    }
  }

  async function deleteConnection(id: string) {
    loading.value = true
    error.value = null
    try {
      await invoke('delete_connection', { id })
      await fetchConnections() // Refresh the list
    } catch (e: any) {
      error.value = e.toString()
    } finally {
      loading.value = false
    }
  }

  function newConnection() {
    editingConnection.value = null
    showConnectionForm.value = true
  }

  function editConnection(connection: SavedConnection) {
    editingConnection.value = connection
    showConnectionForm.value = true
  }

  function cancelConnectionForm() {
    showConnectionForm.value = false
    editingConnection.value = null
  }

  return {
    connections,
    loading,
    error,
    showConnectionForm,
    editingConnection,
    fetchConnections,
    saveConnection,
    deleteConnection,
    newConnection,
    editConnection,
    cancelConnectionForm,
  }
})
