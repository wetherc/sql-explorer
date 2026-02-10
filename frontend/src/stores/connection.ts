import { ref } from 'vue'
import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/tauri'
import type { DbType } from '@/types/savedConnection'

export const useConnectionStore = defineStore('connection', () => {
  const isConnected = ref(false)
  const isConnecting = ref(false)
  const errorMessage = ref('')
  const dbType = ref<DbType | null>(null)

  async function connect(connectionString: string, type: DbType) {
    isConnecting.value = true
    errorMessage.value = ''
    dbType.value = null

    try {
      await invoke('connect', { connectionString, dbType: type })
      isConnected.value = true
      dbType.value = type
    } catch (error) {
      isConnected.value = false
      errorMessage.value = error as string
    } finally {
      isConnecting.value = false
    }
  }

  function disconnect() {
    isConnected.value = false
    dbType.value = null
    // Here we might later want to call a backend command to clean up the connection
  }

  return { isConnected, isConnecting, errorMessage, dbType, connect, disconnect }
})
