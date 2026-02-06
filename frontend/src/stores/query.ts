import { ref, computed } from 'vue'
import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/tauri'
import type { Value as JsonValue } from 'serde_json'

export const useQueryStore = defineStore('query', () => {
  const results = ref<JsonValue | null>(null)
  const isLoading = ref(false)
  const errorMessage = ref('')

  const resultRows = computed(() => {
    if (results.value && Array.isArray(results.value)) {
      return results.value
    }
    return []
  })

  const resultColumns = computed(() => {
    if (resultRows.value.length > 0) {
      const firstRow = resultRows.value[0]
      if (typeof firstRow === 'object' && firstRow !== null) {
        return Object.keys(firstRow)
      }
    }
    return []
  })

  async function executeQuery(query: string) {
    isLoading.value = true
    errorMessage.value = ''
    results.value = null

    try {
      const response = await invoke<JsonValue>('execute_query', { query })
      results.value = response
    } catch (error) {
      errorMessage.value = error as string
    } finally {
      isLoading.value = false
    }
  }

  return {
    results,
    isLoading,
    errorMessage,
    resultRows,
    resultColumns,
    executeQuery,
  }
})
