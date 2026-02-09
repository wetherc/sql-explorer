import { ref, computed } from 'vue'
import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/tauri'
import { type QueryResponse, type ResultSet } from '@/types/query'


export const useQueryStore = defineStore('query', () => {
  const response = ref<QueryResponse | null>(null)
  const isLoading = ref(false)
  const errorMessage = ref('')

  const firstResultSet = computed<ResultSet | null | undefined>(() => {
    return response.value && response.value.results.length > 0
      ? response.value.results[0]
      : null
  })

  const resultRows = computed(() => {
    return firstResultSet.value ? firstResultSet.value.rows : []
  })

  const resultColumns = computed(() => {
    return firstResultSet.value ? firstResultSet.value.columns : []
  })

  const messages = computed(() => {
    return response.value ? response.value.messages : []
  })

  async function executeQuery(query: string): Promise<boolean> {
    isLoading.value = true
    errorMessage.value = ''
    response.value = null

    try {
      const backendResponse = await invoke<QueryResponse>('execute_query', { query })
      response.value = backendResponse
      errorMessage.value = response.value.messages.join('\n')
      // If there's an error message from the backend, consider it a failure.
      // This is a simplified check; more robust error handling might be needed.
      return response.value.messages.every(msg => !msg.toLowerCase().includes('error'))
    } catch (error) {
      errorMessage.value = error as string
      return false
    } finally {
      isLoading.value = false
    }
  }

  // This function is used by the tabs store to set the state of a specific tab.
  function setQueryState(
    newQuery: string, // Not directly used here, managed by tabs store
    newResponse: QueryResponse | null,
    newErrorMessage: string | null,
    newIsLoading: boolean,
  ) {
    response.value = newResponse;
    errorMessage.value = newErrorMessage || '';
    isLoading.value = newIsLoading;
  }

  return {
    response,
    isLoading,
    errorMessage,
    resultRows,
    resultColumns,
    messages,
    executeQuery,
    setQueryState,
  }
})
