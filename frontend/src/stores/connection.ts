// src/stores/connection.ts
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/tauri'
import { DbType } from '@/types/db'

export const useConnectionStore = defineStore('connection', () => {
  const isConnected = ref(false)
  const isConnecting = ref(false)
  const errorMessage = ref<string | null>(null)
  const dbType = ref<DbType | null>(null)

  async function connect(connectionString: string, type: DbType) {
    isConnecting.value = true
    errorMessage.value = null
    isConnected.value = false
    dbType.value = null

    try {
      await invoke('connect', { connectionString, dbType: type })
      isConnected.value = true
      dbType.value = type
    } catch (e: any) {
      errorMessage.value = e.toString()
      isConnected.value = false
    } finally {
      isConnecting.value = false
    }
  }

  function disconnect() {
    isConnected.value = false
    dbType.value = null
    // Future: Call backend to properly close connection
  }

  return {
    isConnected,
    isConnecting,
    errorMessage,
    dbType,
    connect,
    disconnect,
  }
})
