import { defineStore } from 'pinia'
import { markRaw, reactive, ref, toRaw, watch } from 'vue'
import { api } from '@/lib/api'
import { createId } from './connections'
import { useConnectionsStore } from './connections'
import { useHistoryStore } from './history'
import { useSettingsStore } from './settings'
import { useUiStore } from './ui'
import { isCancellation, toErrorPayload } from '@/lib/errors'
import { scanCost } from '@/lib/format'
import { pauseSeconds, releaseKept, spillRequest } from '@/lib/kept'
import { ResultTable, type ResultStreamHandlers } from '@/lib/results'
import { MessageLevel, PlanMode } from '@/types/api'
import { savedFile, type SavedFile } from '@/lib/runFile'
import type {
  ChosenMessagesFile,
  ErrorPayload,
  ExecOptions,
  KeptInfo,
  KeptSet,
  Message,
  QueryStats,
  RunFileSummary,
  UnsavedReason,
  UnsavedSet,
} from '@/types/api'

/** The file of a run to a file, and where its result sets go. */
export interface RunFileTarget {
  /** The ticket that `chooseRunFile` gave. */
  ticket: string
  /** True when each result set goes to the file, not the first alone. */
  eachSet: boolean
}

/** One gigabyte, as a storage unit counts it. */
const BYTES_IN_GIGABYTE = 1024 ** 3

/**
 * The fewest of the last messages of a run that a tab keeps. A loop of the
 * server can send millions of messages, and a list of all of them uses
 * memory without a limit. The list drops its first messages when it gets
 * to twice this length, so it keeps at most twice this count.
 */
export const KEPT_MESSAGES = 2000

/** The file name that the save dialog suggests for messages. */
export const MESSAGES_FILE_NAME = 'messages.txt'

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
  /** The identifier of the kept result of the set, when the row limit cut
   *  the read and the backend can still give every row. The export of all
   *  rows then reads that result and does not run the statement again. */
  keptId?: string
  /** Where the kept rows of the set are, for the grid and the export menu. */
  kept?: KeptInfo
  /** Why the run saved no file for this cut set, when it asked to save its
   *  full results. */
  unsaved?: UnsavedReason
  /** The time the run of the set took, once the run has ended. An export of
   *  all rows that runs the query again takes about as long. */
  elapsedMs?: number
  /** The moment, in milliseconds since the epoch, when the paused read of
   *  the set ends, while the server keeps its statement open. The export of
   *  all rows continues that read. */
  pausedUntil?: number
  /** The statement, the values and the connection that made the result, or
   *  null for a plan. The export of every row runs this again, and not the
   *  text of the editor or the connection that the tab names now, which the
   *  user may have changed since the run. */
  run: PaneRun | null
  /** The file that a run to a file wrote with the rows of this result. */
  savedFile?: SavedFile
}

/** What the export of every row needs to run a statement again. */
export interface PaneRun {
  connectionId: string
  query: string
  params?: Record<string, unknown>
}

/** A place in the text of the editor. Both numbers count from 1. */
export interface EditorPosition {
  line: number
  column: number
}

/** An export of all rows that runs for one tab, so the user can stop it. */
export interface RunningExport {
  connectionId: string
  requestId: string
  stopping: boolean
}

export interface QueryState {
  running: boolean
  /** True when the last run failed. The next run clears it. */
  failed: boolean
  /** True after the user pressed Stop and until the run ends. */
  stopping: boolean
  /** The place of the last failure in the editor, when the server named one. */
  errorLocation: EditorPosition | null
  /** The identifier the backend uses to stop this statement. */
  requestId: string | null
  /** The connection the running statement was sent to. A stop must reach
   *  that connection, whatever the tab names at the moment of the stop. */
  requestConnectionId: string | null
  error: ErrorPayload | null
  panes: ResultPane[]
  /** The last messages of the run. See `KEPT_MESSAGES`. */
  messages: Message[]
  /** The count of the first messages of the run that the tab dropped. */
  droppedMessages: number
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
  /** The export of all rows that is running. The store keeps it, so a view
   *  that mounts again sees it and can't start a second export. */
  exporting: RunningExport | null
  /** The text file that gets every message of each run of the tab, until
   *  the user stops it or closes the tab. */
  messagesFile: ChosenMessagesFile | null
  /** The connection whose session of this tab is inside an open
   *  transaction, or null. Closing the tab rolls that transaction back. */
  openTransactionOn: string | null
  /** The rows and the bytes that the backend saved so far of a set past the
   *  row limit of the grid, while the run saves its full results. */
  saving: { rows: number; bytes: number; stopping: boolean } | null
  /** True while the run reads and drops the rows past the row limit,
   *  because the server can't end the batch early. */
  readingPastLimit: boolean
}

/** Builds the state a tab starts with. */
export function newQueryState(): QueryState {
  return {
    running: false,
    failed: false,
    stopping: false,
    errorLocation: null,
    requestId: null,
    requestConnectionId: null,
    error: null,
    panes: [],
    messages: [],
    droppedMessages: 0,
    rowsAffected: null,
    elapsedMs: 0,
    startedAt: null,
    activePaneId: null,
    lastRunAt: null,
    stats: null,
    exporting: null,
    messagesFile: null,
    openTransactionOn: null,
    saving: null,
    readingPastLimit: false,
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

/**
 * Moves a place in the text that the window sent into the editor. The
 * `start` is the place in the editor where the sent text begins, before
 * the store trims it, and `sent` is that text as the user gave it.
 */
export function editorPosition(
  sent: string,
  start: EditorPosition,
  line: number,
  column: number,
): EditorPosition {
  // The backend counts from the first character of the trimmed text.
  const leading = sent.slice(0, sent.length - sent.trimStart().length).split('\n')
  const lead = leading.length - 1
  const firstColumn = lead === 0 ? start.column : 1
  const textLine = start.line + lead
  const textColumn = firstColumn + leading[lead]!.length
  return line === 1
    ? { line: textLine, column: textColumn + column - 1 }
    : { line: textLine + line - 1, column }
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

/**
 * Adds one message to the end of the list of a tab in place. A copy of the
 * list for each message costs time in the square of their number. At twice
 * `KEPT_MESSAGES` the tab keeps the last `KEPT_MESSAGES` in a new list and
 * counts the others. That copy happens once for each `KEPT_MESSAGES`
 * messages, so the cost of each message stays constant on average.
 */
export function addMessage(state: QueryState, message: Message): void {
  state.messages.push(message)
  if (state.messages.length >= 2 * KEPT_MESSAGES) {
    const all = toRaw(state.messages)
    state.droppedMessages += all.length - KEPT_MESSAGES
    state.messages = all.slice(-KEPT_MESSAGES)
  }
}

/** What a tab says when its server session closed and a new one opened. */
export const SESSION_RESET_TEXT =
  "This tab's session was reset. Temporary tables, open transactions and SET options are gone."

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
    releaseKept(states[tabId]?.panes ?? [])
    const messagesFile = states[tabId]?.messagesFile
    if (messagesFile) {
      void forgetMessagesFile(messagesFile.id)
    }
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

  /**
   * Gives each kept set of a run to the pane of its table. A set whose pane
   * is gone, because the user closed it or the tab, has its kept result
   * released at once.
   */
  function attachKept(state: QueryState, run: Run, kept: KeptSet[]): void {
    for (const entry of kept) {
      const table = run.fresh[entry.set]
      const pane = state.panes.find((pane) => table !== undefined && pane.result === table)
      if (pane) {
        const { origin, keptAt, savedRows, savedBytes } = entry
        pane.keptId = entry.id
        pane.kept = { origin, keptAt, savedRows, savedBytes }
        if (entry.pausedSecs !== undefined) {
          const limit = entry.pausedSecs * 1000
          pane.pausedUntil = Date.now() + limit
          // The backend ends the read at the same time, and the registry
          // then forgets it.
          setTimeout(() => {
            if (pane.keptId === entry.id) {
              endPause(pane)
            }
          }, limit)
        }
      } else {
        releaseKept([{ keptId: entry.id }])
      }
    }
  }

  /**
   * Forgets the paused read of a pane after an export took it, after the
   * user released it, or after its time ran out. With `release`, the
   * backend also lets the read go.
   */
  function endPause(pane: ResultPane, release = false): void {
    if (release) {
      releaseKept([pane])
    }
    forgetKept(pane)
  }

  /**
   * Forgets the kept result of a pane, so an export of all rows runs the
   * query again. The backend no longer has the result, so nothing goes back
   * to it.
   */
  function forgetKept(pane: ResultPane): void {
    pane.keptId = undefined
    pane.kept = undefined
    pane.pausedUntil = undefined
  }

  /** Gives the reason of each cut set that the run saved no file for to
   *  the pane of its table. */
  function attachUnsaved(state: QueryState, run: Run, unsaved: UnsavedSet[]): void {
    for (const entry of unsaved) {
      const table = run.fresh[entry.set]
      const pane = state.panes.find((pane) => table !== undefined && pane.result === table)
      if (pane) {
        pane.unsaved = entry.reason
      }
    }
  }

  function paneOf(state: QueryState, paneId: string): ResultPane | undefined {
    return state.panes.find((pane) => pane.id === paneId)
  }

  /**
   * Runs one statement for one tab and holds what came back.
   *
   * The identifier of the request lets the user stop the statement while it
   * runs. The caller gives the call that reaches the backend, so that a run
   * and a plan share the state of the tab. A run to a file sends its first
   * `savedSets` sets to the file, so a cut of such a set at the row limit
   * gives no warning.
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
    origin?: { sent: string; start: EditorPosition },
    savedSets = 0,
  ): Promise<boolean> {
    const trimmed = query.trim()
    if (trimmed === '') {
      ui.warn('There is nothing to run.')
      return false
    }

    const state = stateFor(tabId)
    if (state.running) {
      ui.warn('A statement is already running in this tab.')
      return false
    }

    const requestId = createId()
    state.running = true
    state.failed = false
    state.stopping = false
    state.errorLocation = null
    state.requestId = requestId
    state.requestConnectionId = connectionId
    state.error = null
    state.readingPastLimit = false
    // A result the user kept stays. Every other result goes. A paused read
    // keeps the session of the tab, so the new run ends it, also for a
    // pinned result.
    releaseKept(state.panes.filter((pane) => !pane.pinned))
    for (const pane of state.panes) {
      if (pane.pinned && pane.pausedUntil !== undefined) {
        endPause(pane, true)
      }
    }
    state.panes = state.panes.filter((pane) => pane.pinned)
    state.messages = []
    state.droppedMessages = 0
    state.rowsAffected = null
    state.elapsedMs = 0
    state.startedAt = Date.now()
    state.activePaneId = null
    state.stats = null
    state.saving = null

    // The history holds the name and not the identifier, so an entry stays
    // readable after the record of the connection is gone.
    const connectionName = connections.nameFor(connectionId)
    let succeeded = false
    let failure: ErrorPayload | null = null
    // The end of the run or its failure says when the session of the tab
    // closed, so the tab tells the user once.
    let sessionReset = false
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
            // The file of the set is complete, or the set saved none.
            state.saving = null
          },
          onMessage: (message) => addMessage(state, message),
          onProgress: ({ rows, bytes }) => {
            state.saving = { rows, bytes, stopping: state.saving?.stopping ?? false }
          },
          onReadingPastLimit: () => {
            state.readingPastLimit = true
          },
          onEnd: (end) => {
            // The end gives the messages that did not stream before it.
            for (const message of end.messages) {
              addMessage(state, message)
            }
            state.rowsAffected = end.rowsAffected
            state.elapsedMs = end.elapsedMs
            state.stats = end.stats
            sessionReset ||= end.sessionReset === true
            if (end.openTransaction !== undefined) {
              state.openTransactionOn = end.openTransaction ? connectionId : null
            }
            for (const pane of state.panes) {
              if (run.fresh.includes(pane.result)) {
                pane.elapsedMs = end.elapsedMs
              }
            }
            attachKept(state, run, end.kept ?? [])
            attachUnsaved(state, run, end.unsaved ?? [])
          },
        },
      )
      recordScan(state.stats)
      succeeded = true
      // A paused result shows its own notice in the grid.
      const paused = new Set(
        state.panes.filter((pane) => pane.pausedUntil !== undefined).map((pane) => pane.result),
      )
      if (run.fresh.slice(savedSets).some((table) => table.truncated && !paused.has(table))) {
        ui.warn('Results stopped at the row limit. Raise the limit in Settings to see more rows.')
      }
    } catch (error) {
      // The messages of the tab hold the same failure, with its whole detail,
      // so the notice in the corner leaves on its own. A tab that closed
      // shows no messages, and the stop that its close sent fails the run,
      // so that failure gives no notice.
      failure = run.abandoned ? toErrorPayload(error) : ui.reportError(error, { kept: true })
      state.error = failure
      sessionReset ||= failure.sessionReset === true
      // A stop that the user asked for is not a failure, so the tab keeps
      // its view and shows no failed mark.
      if (!run.abandoned && !isCancellation(failure)) {
        state.failed = true
        // The messages show the failure, so the view moves to them.
        state.activePaneId = null
        if (origin && typeof failure.line === 'number') {
          state.errorLocation = editorPosition(
            origin.sent,
            origin.start,
            failure.line,
            failure.column ?? 1,
          )
        }
      }
      state.elapsedMs = Date.now() - (state.startedAt ?? Date.now())
    } finally {
      if (!run.abandoned) {
        runs.delete(tabId)
      }
      state.running = false
      state.stopping = false
      state.saving = null
      state.requestId = null
      state.requestConnectionId = null
      state.startedAt = null
      state.readingPastLimit = false
    }
    // A session that closed after the failure rolled its transaction back.
    if (failure?.sessionReset) {
      state.openTransactionOn = null
    }
    if (sessionReset && !run.abandoned) {
      addMessage(state, { level: MessageLevel.Warning, text: SESSION_RESET_TEXT, detail: null })
      ui.warn(SESSION_RESET_TEXT)
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

  /**
   * Runs a statement for one tab. The `start` is the place in the editor
   * where `query` begins, so the store can give the place of a failure in
   * the editor.
   */
  async function execute(
    tabId: string,
    connectionId: string,
    query: string,
    queryParams?: Record<string, unknown>,
    start?: EditorPosition,
    keepAllRows = false,
  ): Promise<boolean> {
    const text = query.trim()
    const spill = spillRequest(settings.settings, keepAllRows || settings.settings.keepFullResults)
    const pauseSecs = pauseSeconds(settings.settings)
    return runRequest(
      tabId,
      connectionId,
      text,
      (requestId, options, handlers) =>
        api.executeQuery(
          {
            connectionId,
            requestId,
            query: text,
            tabId,
            queryParams,
            options,
            spill,
            pauseSecs,
            messagesFile: messagesFileId(tabId),
          },
          handlers,
        ),
      undefined,
      queryParams,
      start ? { sent: query, start } : undefined,
    )
  }

  /**
   * Runs a statement for one tab and writes the rows of its first result
   * set, or of each set, to the file of the ticket. The grid shows the first
   * rows, as in a normal run, and each saved result records its file. Gives
   * back what the files received, or null when the run failed.
   */
  async function runToFile(
    tabId: string,
    connectionId: string,
    query: string,
    target: RunFileTarget,
    queryParams?: Record<string, unknown>,
    start?: EditorPosition,
  ): Promise<RunFileSummary | null> {
    const text = query.trim()
    let summary = null as RunFileSummary | null
    await runRequest(
      tabId,
      connectionId,
      text,
      async (requestId, options, handlers) => {
        summary = await api.runToFile(
          {
            connectionId,
            requestId,
            query: text,
            ticket: target.ticket,
            maxRows: settings.settings.exportRowLimit,
            eachSet: target.eachSet,
            tabId,
            queryParams,
            options,
            messagesFile: messagesFileId(tabId),
          },
          handlers,
        )
      },
      undefined,
      queryParams,
      start ? { sent: query, start } : undefined,
      target.eachSet ? Infinity : 1,
    )
    // A tab that closed during the run has no state and no result.
    const state = peekState(tabId)
    if (summary && state) {
      const panes = panesOfLastRun(state)
      summary.sets.forEach((set, index) => {
        const pane = panes[index]
        if (pane) {
          pane.savedFile = savedFile(set)
        }
      })
    }
    return summary
  }

  /**
   * Reads the plan of one statement and shows it as a result. The actual plan
   * runs the statement, and the caller asks the user before it calls this.
   * The backend measures the place of a failure in the text with the plan
   * keyword in front, so a plan failure gives no place in the editor.
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
      queryParams,
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
        `That statement scanned more than the ${settings.settings.athenaScanWarningGb} GB warning limit.`,
        `Estimated cost: $${cost.toFixed(2)}. You can change the limit in Settings.`,
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
    if (!state.running || state.stopping || !state.requestId || !state.requestConnectionId) {
      return
    }
    state.stopping = true
    try {
      await api.cancelQuery(state.requestConnectionId, state.requestId)
    } catch (error) {
      const payload = toErrorPayload(error)
      state.stopping = false
      ui.warn("Couldn't stop the statement. It may still be running.", payload.message)
    }
  }

  /** The identifier of the file of messages of a tab, when it has one. */
  function messagesFileId(tabId: string): string | undefined {
    return peekState(tabId)?.messagesFile?.id
  }

  /** Stops the writes to a file of messages. A failure goes to the log of
   *  the backend, and the tab no longer names the file. */
  async function forgetMessagesFile(id: string): Promise<void> {
    try {
      await api.forgetMessagesFile(id)
    } catch {
      // The backend writes the failure to its log.
    }
  }

  /**
   * Asks the user for a text file that gets every message of each run of
   * the tab. A run that goes on sends its last kept messages to the file
   * at once, then each new message. The file that the tab used before
   * gets no more messages.
   */
  async function saveAllMessages(tabId: string): Promise<void> {
    try {
      const chosen = await api.chooseMessagesFile(MESSAGES_FILE_NAME)
      if (!chosen) {
        return
      }
      const state = peekState(tabId)
      // The tab closed while the dialog was open.
      if (!state) {
        void forgetMessagesFile(chosen.id)
        return
      }
      const previous = state.messagesFile
      state.messagesFile = chosen
      if (state.running && state.requestId) {
        await api.saveRunMessages(state.requestId, chosen.id)
      }
      if (previous) {
        void forgetMessagesFile(previous.id)
      }
    } catch (error) {
      ui.reportError(error)
    }
  }

  /** Stops the writes of the messages of a tab to its file. */
  function stopSavingMessages(tabId: string): void {
    const state = peekState(tabId)
    if (state?.messagesFile) {
      void forgetMessagesFile(state.messagesFile.id)
      state.messagesFile = null
    }
  }

  /** Asks the user for a path and writes the messages that the tab shows. */
  async function saveShownMessages(tabId: string): Promise<void> {
    const state = stateFor(tabId)
    try {
      const path = await api.saveShownMessages({
        defaultName: MESSAGES_FILE_NAME,
        messages: toRaw(state.messages),
        dropped: state.droppedMessages,
      })
      if (path) {
        ui.success(`Messages saved to ${path}.`)
      }
    } catch (error) {
      ui.reportError(error)
    }
  }

  /**
   * Forgets the open transaction of one tab, because the tab released its
   * session, for example when it moved to another connection.
   */
  function forgetTransaction(tabId: string): void {
    const state = peekState(tabId)
    if (state) {
      state.openTransactionOn = null
    }
  }

  // A disconnect closes every session of the connection, and the server
  // rolls back their transactions.
  watch(
    () => Object.keys(connections.active),
    (open) => {
      for (const state of Object.values(states)) {
        if (state.openTransactionOn !== null && !open.includes(state.openTransactionOn)) {
          state.openTransactionOn = null
        }
      }
    },
  )

  /**
   * Stops the saving of all rows of the running query of a tab. The run goes
   * on and ends at the row limit of the grid.
   */
  async function stopSaving(tabId: string): Promise<void> {
    const state = stateFor(tabId)
    const saving = state.saving
    if (!saving || saving.stopping || !state.requestId) {
      return
    }
    saving.stopping = true
    try {
      await api.stopSaving(state.requestId)
    } catch (error) {
      saving.stopping = false
      ui.warn("Couldn't stop saving the rows.", toErrorPayload(error).message)
    }
  }

  /** Removes the error marker of one tab, for example after an edit. */
  function clearErrorLocation(tabId: string): void {
    const state = peekState(tabId)
    if (state) {
      state.errorLocation = null
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
          `This tab already has ${kept} pinned results. Unpin one, or raise the limit in Settings.`,
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
    releaseKept(state.panes.splice(position, 1))
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
    runToFile,
    explain,
    cancel,
    forgetTransaction,
    clearErrorLocation,
    selectPane,
    togglePin,
    closePane,
    endPause,
    saveAllMessages,
    stopSavingMessages,
    saveShownMessages,
    forgetKept,
    stopSaving,
  }
})
