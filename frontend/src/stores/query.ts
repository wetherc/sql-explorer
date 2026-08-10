import { defineStore } from 'pinia'
import { markRaw, reactive, ref } from 'vue'
import { api } from '@/lib/api'
import { createId } from './connections'
import { useConnectionsStore } from './connections'
import { useHistoryStore } from './history'
import { useSettingsStore } from './settings'
import { useUiStore } from './ui'
import { toErrorPayload } from '@/lib/errors'
import { scanCost } from '@/lib/format'
import { PlanKind } from '@/types/api'
import type {
  ErrorPayload,
  ExecOptions,
  Message,
  QueryResponse,
  QueryStats,
  ResultSet,
} from '@/types/api'

/** One gigabyte, as a storage unit counts it. */
const BYTES_IN_GIGABYTE = 1024 ** 3

/**
 * One result set that the interface shows. The record carries an identifier
 * of its own, because a kept result outlives the run that made it and the
 * position in a list is therefore not an identity.
 */
export interface ResultPane {
  id: string
  result: ResultSet
  /** The place of the set inside the execution that made it, from one. */
  number: number
  /** The moment of the run, which the title of a kept result holds. */
  ranAt: number
  /** True while the user keeps this result against the next run. */
  pinned: boolean
  /** The name of the result, for a result that is not a plain result set. */
  label?: string
}

export interface QueryState {
  running: boolean
  /** The identifier the backend uses to stop this statement. */
  requestId: string | null
  /** The connection the running statement was sent to. A stop must reach
   *  that connection, whatever the tab names at the moment of the stop. */
  requestConnectionId: string | null
  /** The statement and the values of the last run that gave a result. The
   *  export of every row runs this again, and not the text of the editor,
   *  which the user may have changed since the run. */
  lastRun: { query: string; params?: Record<string, unknown> } | null
  error: ErrorPayload | null
  panes: ResultPane[]
  messages: Message[]
  rowsAffected: number | null
  elapsedMs: number
  /** The moment the statement started, so the elapsed time can be shown. */
  startedAt: number | null
  /** The result the user reads, or `null` for the messages. */
  activePaneId: string | null
  /** What the execution cost, for an engine that reports it. */
  stats: QueryStats | null
}

/** Builds the state a tab starts with. */
export function newQueryState(): QueryState {
  return {
    running: false,
    requestId: null,
    requestConnectionId: null,
    lastRun: null,
    error: null,
    panes: [],
    messages: [],
    rowsAffected: null,
    elapsedMs: 0,
    startedAt: null,
    activePaneId: null,
    stats: null,
  }
}

/** Counts the rows of every result set of one execution. */
export function totalRows(results: ResultSet[]): number {
  return results.reduce((sum, result) => sum + result.rows.length, 0)
}

/** Gives the last result of a list, or nothing when the list is empty. */
function lastPane(panes: ResultPane[]): ResultPane | undefined {
  return panes.length > 0 ? panes[panes.length - 1] : undefined
}

/** Gives the result sets of a list of panes, in their order. */
export function resultsOf(panes: ResultPane[]): ResultSet[] {
  return panes.map((pane) => pane.result)
}

export const useQueryStore = defineStore('query', () => {
  const ui = useUiStore()
  const connections = useConnectionsStore()
  const history = useHistoryStore()
  const settings = useSettingsStore()

  const states = reactive<Record<string, QueryState>>({})
  /** The bytes every statement of this session scanned, over all tabs. */
  const sessionScannedBytes = ref(0)

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
   * The number of statements that run against one connection. Closing that
   * connection stops each of them, so the interface asks first when this is
   * more than none.
   */
  function runningOn(connectionId: string): number {
    return Object.values(states).filter(
      (state) => state.running && state.requestConnectionId === connectionId,
    ).length
  }

  function paneOf(state: QueryState, paneId: string): ResultPane | undefined {
    return state.panes.find((pane) => pane.id === paneId)
  }

  /**
   * Runs one statement for one tab and holds what came back.
   *
   * The identifier of the request lets the user stop the statement while it
   * runs. The caller gives the call that reaches the backend, so that a run
   * and a plan share the state of the tab.
   */
  async function runRequest(
    tabId: string,
    connectionId: string,
    query: string,
    call: (requestId: string, options: ExecOptions) => Promise<QueryResponse>,
    label?: string,
    queryParams?: Record<string, unknown>,
  ): Promise<boolean> {
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
    state.requestConnectionId = connectionId
    state.error = null
    // A result the user kept stays. Every other result goes.
    state.panes = state.panes.filter((pane) => pane.pinned)
    state.messages = []
    state.rowsAffected = null
    state.elapsedMs = 0
    state.startedAt = Date.now()
    state.activePaneId = null
    state.stats = null

    // The history holds the name and not the identifier, so an entry stays
    // readable after the record of the connection is gone.
    const connectionName = connections.nameFor(connectionId)
    let succeeded = false
    let failure: ErrorPayload | null = null
    let fresh: ResultSet[] = []

    try {
      const response = await call(requestId, {
        maxRows: settings.settings.maxRows,
        timeoutSecs: connections.byId(connectionId)?.options.queryTimeoutSecs ?? 300,
      })
      const ranAt = Date.now()
      fresh = response.results
      state.panes = [
        ...state.panes,
        ...response.results.map((result, index) => ({
          id: createId(),
          // The rows never change after the run, so the grid does not need
          // a proxy around every row and every cell. The raw form keeps a
          // large result at its own size.
          result: markRaw(result),
          number: index + 1,
          ranAt,
          pinned: false,
          label,
        })),
      ]
      state.activePaneId = lastPane(state.panes)?.id ?? null
      state.messages = response.messages
      state.rowsAffected = response.rowsAffected
      state.elapsedMs = response.elapsedMs
      state.stats = response.stats ?? null
      recordScan(state.stats)
      succeeded = true
      if (label === undefined) {
        state.lastRun = { query: trimmed, params: queryParams }
      }
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
      state.requestConnectionId = null
      state.startedAt = null
    }

    // A plan is not the statement of the user, so the history holds the runs
    // alone.
    if (label === undefined) {
      await history.record({
        connectionId,
        connectionName,
        query: trimmed,
        elapsedMs: state.elapsedMs,
        rowCount: totalRows(fresh),
        succeeded,
        error: failure ? failure.message : null,
      })
    }

    return succeeded
  }

  /** Runs a statement for one tab. */
  function execute(
    tabId: string,
    connectionId: string,
    query: string,
    queryParams?: Record<string, unknown>,
  ): Promise<boolean> {
    const text = query.trim()
    return runRequest(
      tabId,
      connectionId,
      text,
      (requestId, options) =>
        api.executeQuery({ connectionId, requestId, query: text, queryParams, options }),
      undefined,
      queryParams,
    )
  }

  /**
   * Reads the plan of one statement and shows it as a result. The actual plan
   * runs the statement, and the caller asks the user before it calls this.
   */
  function explain(
    tabId: string,
    connectionId: string,
    query: string,
    kind: PlanKind,
    queryParams?: Record<string, unknown>,
  ): Promise<boolean> {
    const text = query.trim()
    return runRequest(
      tabId,
      connectionId,
      text,
      (requestId, options) =>
        api.explainQuery({
          connectionId,
          requestId,
          query: text,
          kind,
          queryParams,
          options,
        }),
      kind === PlanKind.Actual ? 'Actual plan' : 'Estimated plan',
    )
  }

  /**
   * Adds the scan of one execution to the total of the session, and warns
   * when the scan passes the limit in the settings.
   */
  function recordScan(stats: QueryStats | null): void {
    const bytes = stats?.scannedBytes ?? null
    if (bytes === null) {
      return
    }
    sessionScannedBytes.value += bytes
    const limit = settings.settings.athenaScanWarningGb * BYTES_IN_GIGABYTE
    if (bytes > limit) {
      const cost = scanCost(bytes, settings.settings.athenaPricePerTerabyte)
      ui.warn(
        `That statement scanned more than the warning limit of ${settings.settings.athenaScanWarningGb} GB.`,
        `The estimated cost is $${cost.toFixed(2)}. Change the limit in the settings.`,
      )
    }
  }

  /**
   * Asks the backend to stop the statement of one tab. The stop goes to the
   * connection that the statement was sent to, because the user can change
   * the connection of the tab while the statement runs.
   */
  async function cancel(tabId: string): Promise<void> {
    const state = stateFor(tabId)
    if (!state.running || !state.requestId || !state.requestConnectionId) {
      return
    }
    try {
      await api.cancelQuery(state.requestConnectionId, state.requestId)
    } catch (error) {
      const payload = toErrorPayload(error)
      ui.warn(payload.message)
    }
  }

  /** Shows one result, or the messages when the identifier is `null`. */
  function selectPane(tabId: string, paneId: string | null): void {
    const state = stateFor(tabId)
    if (paneId === null || paneOf(state, paneId)) {
      state.activePaneId = paneId
    }
  }

  /**
   * Keeps a result against the next run, or lets it go again. The number of
   * kept results has a limit, because each one holds its rows in memory.
   */
  function togglePin(tabId: string, paneId: string): void {
    const state = stateFor(tabId)
    const pane = paneOf(state, paneId)
    if (!pane) {
      return
    }
    if (!pane.pinned) {
      const kept = state.panes.filter((item) => item.pinned).length
      if (kept >= settings.settings.maxPinnedResults) {
        ui.warn(
          `This tab already keeps ${kept} results. Close one, or raise the limit in the settings.`,
        )
        return
      }
    }
    pane.pinned = !pane.pinned
  }

  /** Closes one result. The next result takes its place in the view. */
  function closePane(tabId: string, paneId: string): void {
    const state = stateFor(tabId)
    const position = state.panes.findIndex((pane) => pane.id === paneId)
    if (position < 0) {
      return
    }
    state.panes.splice(position, 1)
    if (state.activePaneId === paneId) {
      const next = state.panes[position] ?? lastPane(state.panes) ?? null
      state.activePaneId = next ? next.id : null
    }
  }

  return {
    states,
    sessionScannedBytes,
    stateFor,
    clear,
    runningOn,
    execute,
    explain,
    cancel,
    selectPane,
    togglePin,
    closePane,
  }
})
