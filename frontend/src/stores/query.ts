import { defineStore } from 'pinia'
import { reactive } from 'vue'
import { api } from '@/lib/api'
import { createId } from './connections'
import { useConnectionsStore } from './connections'
import { useHistoryStore } from './history'
import { useSettingsStore } from './settings'
import { useUiStore } from './ui'
import { toErrorPayload } from '@/lib/errors'
import type { ErrorPayload, ResultSet } from '@/types/api'

export interface QueryState {
  running: boolean
  /** The identifier the backend uses to stop this statement. */
  requestId: string | null
  error: ErrorPayload | null
  results: ResultSet[]
  messages: string[]
  rowsAffected: number | null
  elapsedMs: number
  /** The moment the statement started, so the elapsed time can be shown. */
  startedAt: number | null
  activeResultIndex: number
}

/** Builds the state a tab starts with. */
export function newQueryState(): QueryState {
  return {
    running: false,
    requestId: null,
    error: null,
    results: [],
    messages: [],
    rowsAffected: null,
    elapsedMs: 0,
    startedAt: null,
    activeResultIndex: 0,
  }
}

/** Counts the rows of every result set of one execution. */
export function totalRows(results: ResultSet[]): number {
  return results.reduce((sum, result) => sum + result.rows.length, 0)
}

export const useQueryStore = defineStore('query', () => {
  const ui = useUiStore()
  const connections = useConnectionsStore()
  const history = useHistoryStore()
  const settings = useSettingsStore()

  const states = reactive<Record<string, QueryState>>({})

  function stateFor(tabId: string): QueryState {
    if (!states[tabId]) {
      states[tabId] = newQueryState()
    }
    return states[tabId]
  }

  function clear(tabId: string): void {
    delete states[tabId]
  }

  /**
   * Runs a statement for one tab. The identifier of the request lets the
   * user stop the statement while it runs.
   */
  async function execute(tabId: string, connectionId: string, query: string): Promise<boolean> {
    const trimmed = query.trim()
    if (trimmed === '') {
      ui.warn('There is no statement to run.')
      return false
    }

    const state = stateFor(tabId)
    if (state.running) {
      ui.warn('This tab already runs a statement.')
      return false
    }

    const requestId = createId()
    state.running = true
    state.requestId = requestId
    state.error = null
    state.results = []
    state.messages = []
    state.rowsAffected = null
    state.elapsedMs = 0
    state.startedAt = Date.now()
    state.activeResultIndex = 0

    const connectionName = connections.byId(connectionId)?.name ?? connectionId
    let succeeded = false
    let failure: ErrorPayload | null = null

    try {
      const response = await api.executeQuery({
        connectionId,
        requestId,
        query: trimmed,
        options: {
          maxRows: settings.settings.maxRows,
          timeoutSecs: connections.byId(connectionId)?.options.queryTimeoutSecs ?? 300,
        },
      })
      state.results = response.results
      state.messages = response.messages
      state.rowsAffected = response.rowsAffected
      state.elapsedMs = response.elapsedMs
      succeeded = true
      if (response.results.some((result) => result.truncated)) {
        ui.warn('The row limit stopped the read. Raise it in the settings to see more rows.')
      }
    } catch (error) {
      failure = ui.reportError(error)
      state.error = failure
      state.elapsedMs = Date.now() - (state.startedAt ?? Date.now())
    } finally {
      state.running = false
      state.requestId = null
      state.startedAt = null
    }

    await history.record({
      connectionId,
      connectionName,
      query: trimmed,
      elapsedMs: state.elapsedMs,
      rowCount: totalRows(state.results),
      succeeded,
      error: failure ? failure.message : null,
    })

    return succeeded
  }

  /** Asks the backend to stop the statement of one tab. */
  async function cancel(tabId: string, connectionId: string): Promise<void> {
    const state = stateFor(tabId)
    if (!state.running || !state.requestId) {
      return
    }
    try {
      await api.cancelQuery(connectionId, state.requestId)
    } catch (error) {
      const payload = toErrorPayload(error)
      ui.warn(payload.message)
    }
  }

  function selectResult(tabId: string, index: number): void {
    const state = stateFor(tabId)
    if (index >= 0 && index < state.results.length) {
      state.activeResultIndex = index
    }
  }

  return { states, stateFor, clear, execute, cancel, selectResult }
})
