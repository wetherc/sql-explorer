import { ref } from 'vue'
import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/tauri'

export const useConnectionStore = defineStore('connection', () => {
  const isConnected = ref(false)
  const isConnecting = ref(false)
  const errorMessage = ref('')

  async function connect(connectionString: string) {
    isConnecting.value = true
    errorMessage.value = ''

    try {
      await invoke('connect', { connectionString })
      isConnected.value = true
    } catch (error) {
      isConnected.value = false
      errorMessage.value = error as string
    } finally {
      isConnecting.value = false
    }
  }

  function disconnect() {
    isConnected.value = false
    // Here we might later want to call a backend command to clean up the connection
  }

  return { isConnected, isConnecting, errorMessage, connect, disconnect }
})
