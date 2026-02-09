import { ref, computed } from 'vue'
import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/tauri'

type JsonValue = string | number | boolean | null | any[] | { [key: string]: any }

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

  async function executeQuery(query: string): Promise<boolean> {
    isLoading.value = true
    errorMessage.value = ''
    results.value = null

    try {
      const response = await invoke<JsonValue>('execute_query', { query })
      results.value = response
      return true
    } catch (error) {
      errorMessage.value = error as string
      return false
    } finally {
      isLoading.value = false
    }
  }

  function setQueryState(
    newQuery: string,
    newColumns: string[],
    newRows: Record<string, any>[],
    newErrorMessage: string | null,
    newIsLoading: boolean,
  ) {
    // Note: The newQuery is not directly stored in the query store,
    // as it is managed by the active tab in the tabs store.
    // This function primarily updates the results, loading, and error states.
    results.value = { columns: newColumns, rows: newRows }; // Simplified structure for direct assignment
    errorMessage.value = newErrorMessage || '';
    isLoading.value = newIsLoading;
  }

  return {
    results,
    isLoading,
    errorMessage,
    resultRows,
    resultColumns,
    executeQuery,
    setQueryState,
  }
})
