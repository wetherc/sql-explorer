import { ref } from 'vue'
import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/tauri'
import type { SavedConnection } from '@/types/savedConnection'

export const useSavedConnectionsStore = defineStore('savedConnections', () => {
  const connections = ref<SavedConnection[]>([])
  const isLoading = ref(false)
  const errorMessage = ref('')

  async function fetchConnections() {
    isLoading.value = true
    errorMessage.value = ''
    try {
      connections.value = await invoke('list_connections')
    } catch (error) {
      errorMessage.value = error as string
    } finally {
      isLoading.value = false
    }
  }

  async function saveConnection(connection: SavedConnection, password?: string) {
    isLoading.value = true
    errorMessage.value = ''
    try {
      await invoke('save_connection', {
        name: connection.name,
        dbType: connection.db_type,
        server: connection.server,
        database: connection.database,
        authType: connection.auth_type,
        user: connection.user,
        password: password
      })
      // Refresh the list
      await fetchConnections()
    } catch (error) {
      errorMessage.value = error as string
      // Re-throw the error to be caught by the caller if needed
      throw error
    } finally {
      isLoading.value = false
    }
  }

  async function deleteConnection(name: string) {
    isLoading.value = true
    errorMessage.value = ''
    try {
      await invoke('delete_connection', { name })
      // Refresh the list
      await fetchConnections()
    } catch (error) {
      errorMessage.value = error as string
    } finally {
      isLoading.value = false
    }
  }

  async function getPassword(name: string): Promise<string | null> {
    try {
      const password = await invoke('get_connection_password', { name })
      return password as string
    } catch (error) {
      errorMessage.value = error as string
      return null
    }
  }

  return {
    connections,
    isLoading,
    errorMessage,
    fetchConnections,
    saveConnection,
    deleteConnection,
    getPassword
  }
})
