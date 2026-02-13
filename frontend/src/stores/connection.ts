// src/stores/connection.ts
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/tauri'
import type { SavedConnection } from '@/stores/connectionManager'

export const useConnectionStore = defineStore('connection', () => {
  const isConnecting = ref(false)
  const errorMessage = ref<string | null>(null)
  const activeConnections = ref<Record<string, SavedConnection>>({})

  const isConnected = computed(() => Object.keys(activeConnections.value).length > 0)

  async function connect(savedConnection: SavedConnection) {
    isConnecting.value = true
    errorMessage.value = null

    // Construct connection string based on dbType and fields
    let connectionString = ''
    switch (savedConnection.dbType) {
      case 'mssql':
        connectionString = `server=${savedConnection.host};port=${savedConnection.port};user=${savedConnection.user};TrustServerCertificate=true;`
        if (savedConnection.password) {
          connectionString += `;password=${savedConnection.password}`
        }
        if (savedConnection.database) {
          connectionString += `;database=${savedConnection.database}`
        }
        break
      case 'mysql':
        connectionString = `mysql://${savedConnection.user}`
        if (savedConnection.password) {
          connectionString += `:${savedConnection.password}`
        }
        connectionString += `@${savedConnection.host}:${savedConnection.port}`
        if (savedConnection.database) {
          connectionString += `/${savedConnection.database}`
        }
        break
      case 'postgres':
        connectionString = `postgresql://${savedConnection.user}`
        if (savedConnection.password) {
          connectionString += `:${savedConnection.password}`
        }
        connectionString += `@${savedConnection.host}:${savedConnection.port}`
        if (savedConnection.database) {
          connectionString += `/${savedConnection.database}`
        }
        break
    }

    try {
      await invoke('connect', {
        connectionId: savedConnection.id,
        connectionString,
        dbType: savedConnection.dbType,
      })
      activeConnections.value[savedConnection.id] = savedConnection
    } catch (e: any) {
      errorMessage.value = e.toString()
      delete activeConnections.value[savedConnection.id]
    } finally {
      isConnecting.value = false
    }
  }

  async function disconnect(connectionId: string) {
    try {
      await invoke('disconnect', { connectionId })
      delete activeConnections.value[connectionId]
    } catch (e: any) {
      errorMessage.value = e.toString()
    }
  }

  return {
    isConnecting,
    errorMessage,
    activeConnections,
    isConnected,
    connect,
    disconnect,
  }
})

