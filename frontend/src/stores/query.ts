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
import { ResultTable, type ResultStreamHandlers } from '@/lib/results'
import { PlanMode } from '@/types/api'
import type { ErrorPayload, ExecOptions, Message, QueryStats } from '@/types/api'

/** One gigabyte, as a storage unit counts it. */
const BYTES_IN_GIGABYTE = 1024 ** 3

/**
 * One result set that the interface shows. The record carries an identifier
 * of its own, because a kept result outlives the run that made it and the
 * position in a list is therefore not an identity.
 */
export interface ResultPane {
  id: string
  result: ResultTable
  /** The rows the table holds. The table itself stands outside the deep
   *  reactivity of Vue, so this count tells the grid that rows arrived
   *  while the set streams. */
  rows: number
  /** True when the row limit stopped the read. The table sets its own mark
   *  when the set ends, outside the reactivity of Vue, so this copy tells
   *  the grid. */
  truncated: boolean
  /** The place of the set inside the execution that made it, from one. */
  number: number
  /** The moment of the run, which the title of a kept result holds. */
  ranAt: number
  /** True while the user keeps this result against the next run. */
  pinned: boolean
  /** The name of the result, for a result that is not a plain result set. */
  label?: string
  /** The statement, the values and the connection that made the result, or
   *  null for a plan. The export of every row runs this again, and not the
   *  text of the editor or the connection that the tab names now, which the
   *  user may have changed since the run. */
  run: PaneRun | null
}

/** What the export of every row needs to run a statement again. */
export interface PaneRun {
  connectionId: string
  query: string
  params?: Record<string, unknown>
}

export interface QueryState {
  running: boolean
  /** The identifier the backend uses to stop this statement. */
  requestId: string | null
  /** The connection the running statement was sent to. A stop must reach
   *  that connection, whatever the tab names at the moment of the stop. */
  requestConnectionId: string | null
  error: ErrorPayload | null
  panes: ResultPane[]
  messages: Message[]
  rowsAffected: number | null
  elapsedMs: number
  /** The moment the statement started, so the elapsed time can be shown. */
  startedAt: number | null
  /** The result the user reads, or `null` for the messages. */
  activePaneId: string | null
  /** The moment of the last run. A result that carries this moment came
   *  from that run, and every other result is one the user kept. */
  lastRunAt: number | null
  /** What the execution cost, for an engine that reports it. */
  stats: QueryStats | null
}

/** Builds the state a tab starts with. */
export function newQueryState(): QueryState {
  return {
    running: false,
    requestId: null,
    requestConnectionId: null,
    error: null,
    panes: [],
    messages: [],
    rowsAffected: null,
    elapsedMs: 0,
    startedAt: null,
    activePaneId: null,
    lastRunAt: null,
    stats: null,
  }
}

/**
 * The row limit of one run. The settings and the connection each hold a
 * limit, and the smaller one holds, so a connection can keep its results
 * smaller than the other connections. A connection limit that is not a
 * positive number counts as absent.
 */
export function runRowLimit(settingsLimit: number, connectionLimit: number | undefined): number {
  return connectionLimit !== undefined && connectionLimit > 0
    ? Math.min(settingsLimit, connectionLimit)
    : settingsLimit
}

/** Counts the rows of every result set of one execution. */
export function totalRows(results: ResultTable[]): number {
  return results.reduce((sum, result) => sum + result.rowCount, 0)
}

/** The tables of one run, and whether the tab of the run has closed. */
interface Run {
  fresh: ResultTable[]
  abandoned: boolean
  /** The rows of the run at the moment that its tab closed. */
  rowsAtClose: number
}

/** Gives the last result of a list, or nothing when the list is empty. */
function lastPane(panes: ResultPane[]): ResultPane | undefined {
  return panes.length > 0 ? panes[panes.length - 1] : undefined
}

/**
 * Gives the results that the last run made. A result that the user kept
 * against the next run carries the moment of an older run, so it stays out
 * and the numbers of the status bar hold for the last run alone.
 */
export function panesOfLastRun(state: QueryState): ResultPane[] {
  return state.panes.filter((pane) => pane.ranAt === state.lastRunAt)
}

export const useQueryStore = defineStore('query', () => {
  const ui = useUiStore()
  const connections = useConnectionsStore()
  const history = useHistoryStore()
  const settings = useSettingsStore()

  const states = reactive<Record<string, QueryState>>({})
  /** The run of each tab that runs a statement. */
  const runs = new Map<string, Run>()
  /** The bytes every statement of this session scanned, over all tabs. */
  const sessionScannedBytes = ref(0)

  /**
   * The state of one tab, when the tab has one. The reader of a view calls
   * this, because it writes nothing and a view must not write into the
   * store while it renders.
   */
  function peekState(tabId: string): QueryState | undefined {
    return states[tabId]
  }

  function stateFor(tabId: string): QueryState {
    if (!states[tabId]) {
      states[tabId] = newQueryState()
    }
    return states[tabId]
  }

  /**
   * Forgets the state of a tab that closed. A run of that tab goes on until
   * the backend answers, so the store marks it abandoned. The run then opens
   * no result, and the tables it read go now and not when the backend
   * answers. The close of the tab sends the stop of the statement through
   * `cancel` before it calls this.
   */
  function clear(tabId: string): void {
    const run = runs.get(tabId)
    if (run) {
      run.abandoned = true
      run.rowsAtClose = totalRows(run.fresh)
      run.fresh = []
      // A run stands in the map only while the state of its tab exists.
      states[tabId]!.panes = []
      runs.delete(tabId)
    }
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
    call: (
      requestId: string,
      options: ExecOptions,
      handlers: ResultStreamHandlers,
    ) => Promise<void>,
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
    const run: Run = { fresh: [], abandoned: false, rowsAtClose: 0 }
    runs.set(tabId, run)

    try {
      const ranAt = Date.now()
      state.lastRunAt = ranAt
      const openPane = (table: ResultTable): void => {
        if (run.abandoned) {
          return
        }
        run.fresh.push(table)
        const pane: ResultPane = {
          id: createId(),
          // The table holds the bytes of the rows and changes only while
          // the set streams, so the raw form keeps it at its own size and
          // Vue builds no proxy around it. The count of the rows beside it
          // carries the growth to the grid.
          result: markRaw(table),
          rows: table.rowCount,
          truncated: table.truncated,
          number: run.fresh.length,
          ranAt,
          pinned: false,
          label,
          run: label === undefined ? { connectionId, query: trimmed, params: queryParams } : null,
        }
        state.panes = [...state.panes, pane]
        state.activePaneId = pane.id
      }
      await call(
        requestId,
        {
          maxRows: runRowLimit(
            settings.settings.maxRows,
            connections.byId(connectionId)?.options.maxRows,
          ),
          timeoutSecs: connections.byId(connectionId)?.options.queryTimeoutSecs ?? 300,
        },
        {
          // A set stands in the interface as soon as it opens, and its rows
          // appear while the set streams.
          onBegin: (table) => openPane(table),
          onRows: (table) => {
            const pane = state.panes.find((pane) => pane.result === table)
            if (pane) {
              pane.rows = table.rowCount
            }
          },
          onSet: (table) => {
            // A plan and the stub of a test give the set whole, with no
            // frame that opens it, so the pane opens here instead.
            const pane = state.panes.find((pane) => pane.result === table)
            if (pane) {
              pane.rows = table.rowCount
              pane.truncated = table.truncated
            } else {
              openPane(table)
            }
          },
          onEnd: (end) => {
            state.messages = end.messages
            state.rowsAffected = end.rowsAffected
            state.elapsedMs = end.elapsedMs
            state.stats = end.stats
          },
        },
      )
      recordScan(state.stats)
      succeeded = true
      if (run.fresh.some((table) => table.truncated)) {
        ui.warn('The row limit stopped the read. Raise it in the settings to see more rows.')
      }
    } catch (error) {
      // The messages of the tab hold the same failure, with its whole detail,
      // so the notice in the corner leaves on its own. A tab that closed
      // shows no messages, and the stop that its close sent fails the run,
      // so that failure gives no notice.
      failure = run.abandoned ? toErrorPayload(error) : ui.reportError(error, { kept: true })
      state.error = failure
      state.elapsedMs = Date.now() - (state.startedAt ?? Date.now())
    } finally {
      if (!run.abandoned) {
        runs.delete(tabId)
      }
      state.running = false
      state.requestId = null
      state.requestConnectionId = null
      state.startedAt = null
    }

    // A plan is not the statement of the user, so the history holds the runs
    // alone. The history also holds a run whose tab closed, because the
    // statement reached the server and can have changed data there.
    if (label === undefined) {
      await history.record({
        connectionId,
        connectionName,
        query: trimmed,
        elapsedMs: state.elapsedMs,
        rowCount: run.abandoned ? run.rowsAtClose : totalRows(run.fresh),
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
      (requestId, options, handlers) =>
        api.executeQuery(
          { connectionId, requestId, query: text, tabId, queryParams, options },
          handlers,
        ),
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
    mode: PlanMode,
    queryParams?: Record<string, unknown>,
  ): Promise<boolean> {
    const text = query.trim()
    return runRequest(
      tabId,
      connectionId,
      text,
      // A plan holds few rows, so it comes back as one answer. The store
      // gives those rows to the same handlers as a run.
      async (requestId, options, handlers) => {
        const response = await api.explainQuery({
          connectionId,
          requestId,
          query: text,
          mode,
          tabId,
          queryParams,
          options,
        })
        for (const result of response.results) {
          handlers.onSet(ResultTable.fromRows(result.columns, result.rows, result.truncated))
        }
        handlers.onEnd({
          messages: response.messages,
          rowsAffected: response.rowsAffected,
          elapsedMs: response.elapsedMs,
          stats: response.stats ?? null,
        })
      },
      mode === PlanMode.Actual ? 'Actual plan' : 'Estimated plan',
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
    peekState,
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
