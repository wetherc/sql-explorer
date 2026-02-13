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
      case 'Mssql':
        connectionString = `server=${savedConnection.host};port=${savedConnection.port};user=${savedConnection.user};database=${savedConnection.database};TrustServerCertificate=true;`
        break
      case 'Mysql':
        connectionString = `mysql://${savedConnection.user}@${savedConnection.host}:${savedConnection.port}/${savedConnection.database}`
        break
      case 'Postgres':
        connectionString = `postgresql://${savedConnection.user}@${savedConnection.host}:${savedConnection.port}/${savedConnection.database}`
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
