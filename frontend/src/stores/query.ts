// src/stores/query.ts
import { defineStore } from 'pinia'
import { ref, reactive } from 'vue'
import { invoke } from '@tauri-apps/api/tauri'

export interface QueryResult {
  columns: { title: string, key: string }[]
  rows: any[]
}

export interface QueryState {
  loading: boolean
  error: string | null
  results: QueryResult[]
  messages: string[]
}

export const useQueryStore = defineStore('query', () => {
  const queryStates = reactive<Map<string, QueryState>>(new Map())

  function getStateForTab(tabId: string): QueryState {
    if (!queryStates.has(tabId)) {
      queryStates.set(tabId, {
        loading: false,
        error: null,
        results: [],
        messages: [],
      })
    }
    return queryStates.get(tabId)!
  }

  async function executeQuery(tabId: string, query: string) {
    const state = getStateForTab(tabId)
    state.loading = true
    state.error = null
    state.results = []
    state.messages = []

    try {
      const response = await invoke<{ results: any[], messages: string[] }>('execute_query', { query })
      
      state.results = response.results.map(rs => ({
        columns: rs.columns.map((col: string) => ({ title: col, key: col })),
        rows: rs.rows,
      }))
      state.messages = response.messages

    } catch (e: any) {
      state.error = e.toString()
    } finally {
      state.loading = false
    }
  }

  function clearStateForTab(tabId: string) {
    if (queryStates.has(tabId)) {
      queryStates.delete(tabId)
    }
  }

  return {
    queryStates,
    getStateForTab,
    executeQuery,
    clearStateForTab,
  }
})
