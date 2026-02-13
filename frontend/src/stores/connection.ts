// src/stores/connection.ts
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/tauri'
import { DbType } from '@/types/db'
import type { SavedConnection } from '@/stores/connectionManager'

export const useConnectionStore = defineStore('connection', () => {
  const isConnected = ref(false)
  const isConnecting = ref(false)
  const errorMessage = ref<string | null>(null)
  const activeConnection = ref<SavedConnection | null>(null)

  async function connect(savedConnection: SavedConnection) {
    isConnecting.value = true
    errorMessage.value = null
    isConnected.value = false
    activeConnection.value = null

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
      await invoke('connect', { connectionString, dbType: savedConnection.dbType })
      isConnected.value = true
      activeConnection.value = savedConnection
    } catch (e: any) {
      errorMessage.value = e.toString()
      isConnected.value = false
    } finally {
      isConnecting.value = false
    }
  }

  function disconnect() {
    isConnected.value = false
    activeConnection.value = null
    // Future: Call backend to properly close connection
  }

  return {
    isConnected,
    isConnecting,
    errorMessage,
    activeConnection,
    connect,
    disconnect,
  }
})
