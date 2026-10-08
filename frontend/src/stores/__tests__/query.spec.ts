import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub, connectionFixture, infoFixture, streamed } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const {
  KEPT_MESSAGES,
  SESSION_RESET_TEXT,
  editorPosition,
  newQueryState,
  runRowLimit,
  totalRows,
  useQueryStore,
} = await import('@/stores/query')
const { ResultTable } = await import('@/lib/results')
const { useConnectionsStore } = await import('@/stores/connections')
const { useHistoryStore } = await import('@/stores/history')
const { useSettingsStore } = await import('@/stores/settings')
const { useUiStore } = await import('@/stores/ui')

function response(rows = [[1]]) {
  return {
    results: [
      {
        columns: [{ name: 'n', typeName: 'int' }],
        rows,
        truncated: false,
      },
    ],
    messages: [{ level: 'info' as const, text: '1 row returned.', detail: null }],
    rowsAffected: null,
    elapsedMs: 12,
  }
}

/** A response with two result sets, for the tests of the kept results. */
function twoResults() {
  return {
    ...response(),
    results: [
      { columns: [], rows: [[1]], truncated: false },
      { columns: [], rows: [[2]], truncated: false },
    ],
  }
}

describe('newQueryState', () => {
  it('starts at rest', () => {
    expect(newQueryState()).toEqual({
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
    })
  })
})

describe('editorPosition', () => {
  it('adds the start of the sent text to a place inside it', () => {
    const start = { line: 4, column: 3 }
    expect(editorPosition('SELECT x', start, 1, 8)).toEqual({ line: 4, column: 10 })
    expect(editorPosition('SELECT 1;\nSELECT x', start, 2, 8)).toEqual({ line: 5, column: 8 })
  })

  it('counts the blank text that the store trims away', () => {
    const start = { line: 4, column: 3 }
    expect(editorPosition('  SELECT x', start, 1, 8)).toEqual({ line: 4, column: 12 })
    expect(editorPosition('\n\n  SELECT x', start, 1, 8)).toEqual({ line: 6, column: 10 })
    expect(editorPosition('\n SELECT 1;\nSELECT x', start, 2, 8)).toEqual({
      line: 6,
      column: 8,
    })
  })
})

describe('totalRows', () => {
  it('counts the rows of every result set', () => {
    expect(totalRows([])).toBe(0)
    expect(totalRows([ResultTable.fromRows([], [[1], [2]]), ResultTable.fromRows([], [[3]])])).toBe(
      3,
    )
  })
})

describe('runRowLimit', () => {
  it('keeps the smaller of the two limits', () => {
    expect(runRowLimit(10000, 500)).toBe(500)
    expect(runRowLimit(25, 10000)).toBe(25)
    expect(runRowLimit(25, undefined)).toBe(25)
    expect(runRowLimit(25, 0)).toBe(25)
  })
})

describe('query store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.addHistoryEntry.mockResolvedValue([])
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([])
  })

  it('gives each tab its own state', () => {
    const queries = useQueryStore()
    const first = queries.stateFor('t1')
    expect(queries.stateFor('t1')).toBe(first)
    expect(queries.stateFor('t2')).not.toBe(first)
  })

  it('forgets the state of a tab that closed', () => {
    const queries = useQueryStore()
    queries.stateFor('t1')
    queries.clear('t1')
    expect(Object.keys(queries.states)).toEqual([])
  })

  /**
   * A run that opens a set of two rows and then waits. The release lets it
   * give a second set, which the row limit stopped, and then end or fail.
   */
  function heldRun() {
    let release: (failure?: unknown) => void = () => {}
    apiStub.executeQuery.mockImplementation(
      async (_request: unknown, handlers: import('@/lib/results').ResultStreamHandlers) => {
        handlers.onBegin?.(ResultTable.fromRows([], [[1], [2]]))
        const failure = await new Promise((resolve) => {
          release = resolve
        })
        handlers.onSet(ResultTable.fromRows([], [[3]], true))
        if (failure !== undefined) {
          throw failure
        }
        handlers.onEnd({ messages: [], rowsAffected: null, elapsedMs: 5, stats: null })
      },
    )
    return { release: (failure?: unknown) => release(failure) }
  }

  it('drops the results of a run whose tab closed, and keeps it in the history', async () => {
    const held = heldRun()
    const record = vi.spyOn(useHistoryStore(), 'record')
    const ui = useUiStore()
    const warn = vi.spyOn(ui, 'warn')
    const queries = useQueryStore()
    const running = queries.execute('t1', 'c1', 'SELECT 1')
    await Promise.resolve()
    const state = queries.stateFor('t1')
    expect(state.panes).toHaveLength(1)

    queries.clear('t1')
    expect(Object.keys(queries.states)).toEqual([])
    expect(state.panes).toEqual([])

    held.release()
    expect(await running).toBe(true)
    // The set that came after the close opens no result.
    expect(state.panes).toEqual([])
    expect(warn).not.toHaveBeenCalled()
    expect(Object.keys(queries.states)).toEqual([])
    expect(record).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1', rowCount: 2, succeeded: true }),
    )
    // A new run of a tab of the same name is a run of its own.
    apiStub.executeQuery.mockImplementation(streamed(response()))
    expect(await queries.execute('t1', 'c1', 'SELECT 2')).toBe(true)
    expect(queries.stateFor('t1').panes).toHaveLength(1)
  })

  it('gives no notice for the failure of a run whose tab closed', async () => {
    const held = heldRun()
    const record = vi.spyOn(useHistoryStore(), 'record')
    const ui = useUiStore()
    const report = vi.spyOn(ui, 'reportError')
    const queries = useQueryStore()
    const running = queries.execute('t1', 'c1', 'SELECT 1')
    await Promise.resolve()
    queries.clear('t1')

    const warn = vi.spyOn(ui, 'warn')
    held.release({
      category: 'cancelled',
      message: 'The statement was stopped.',
      detail: null,
      sessionReset: true,
    })
    expect(await running).toBe(false)
    expect(report).not.toHaveBeenCalled()
    // A tab that closed gets no word of its new session.
    expect(warn).not.toHaveBeenCalled()
    expect(record).toHaveBeenCalledWith(
      expect.objectContaining({
        rowCount: 2,
        succeeded: false,
        error: 'The statement was stopped.',
      }),
    )
  })

  it('keeps the view and sets no failed mark when the user stops a run', async () => {
    const held = heldRun()
    const queries = useQueryStore()
    const running = queries.execute('t1', 'c1', 'SELECT 1', undefined, { line: 1, column: 1 })
    await Promise.resolve()
    const state = queries.stateFor('t1')

    held.release({
      category: 'cancelled',
      message: 'The statement was stopped.',
      detail: null,
      line: 1,
    })
    expect(await running).toBe(false)
    expect(state.failed).toBe(false)
    // The result that the run opened last stays in view.
    expect(state.activePaneId).toBe(state.panes[1]?.id)
    expect(state.errorLocation).toBeNull()
    expect(state.error?.category).toBe('cancelled')
  })

  it('tells the tab when a stop reset its session', async () => {
    const held = heldRun()
    const warn = vi.spyOn(useUiStore(), 'warn')
    const queries = useQueryStore()
    const running = queries.execute('t1', 'c1', 'SELECT 1')
    await Promise.resolve()
    held.release({
      category: 'cancelled',
      message: 'The statement was stopped.',
      detail: null,
      sessionReset: true,
    })
    expect(await running).toBe(false)
    const state = queries.stateFor('t1')
    expect(state.messages[state.messages.length - 1]).toEqual({
      level: 'warning',
      text: SESSION_RESET_TEXT,
      detail: null,
    })
    expect(warn).toHaveBeenCalledWith(SESSION_RESET_TEXT)
  })

  it('tells the tab when its session opened again before the run', async () => {
    apiStub.executeQuery.mockImplementation(
      async (_request: unknown, handlers: import('@/lib/results').ResultStreamHandlers) => {
        handlers.onEnd({
          messages: [],
          rowsAffected: null,
          elapsedMs: 1,
          stats: null,
          sessionReset: true,
        })
      },
    )
    const warn = vi.spyOn(useUiStore(), 'warn')
    const queries = useQueryStore()
    expect(await queries.execute('t1', 'c1', 'SELECT 1')).toBe(true)
    expect(queries.stateFor('t1').messages.map((message) => message.text)).toEqual([
      SESSION_RESET_TEXT,
    ])
    expect(warn).toHaveBeenCalledTimes(1)

    // A run on the same session says nothing more.
    apiStub.executeQuery.mockImplementation(streamed(response()))
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(warn).toHaveBeenCalledTimes(1)
  })

  it('keeps the open transaction that the end of a run reports', async () => {
    const ends = [{ openTransaction: true }, {}, { openTransaction: false }]
    apiStub.executeQuery.mockImplementation(
      async (_request: unknown, handlers: import('@/lib/results').ResultStreamHandlers) => {
        handlers.onEnd({
          messages: [],
          rowsAffected: null,
          elapsedMs: 1,
          stats: null,
          ...ends.shift(),
        })
      },
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'BEGIN')
    expect(queries.stateFor('t1').openTransactionOn).toBe('c1')
    // An end that doesn't know the state changes nothing.
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(queries.stateFor('t1').openTransactionOn).toBe('c1')
    await queries.execute('t1', 'c1', 'COMMIT')
    expect(queries.stateFor('t1').openTransactionOn).toBeNull()
  })

  it('forgets the open transaction of a session that a stop reset', async () => {
    const held = heldRun()
    const queries = useQueryStore()
    queries.stateFor('t1').openTransactionOn = 'c1'
    const running = queries.execute('t1', 'c1', 'UPDATE t SET a = 1')
    await Promise.resolve()
    held.release({ category: 'timeout', message: 'Timed out.', detail: null, sessionReset: true })
    await running
    expect(queries.stateFor('t1').openTransactionOn).toBeNull()

    // A failure that keeps the session keeps the transaction.
    const again = heldRun()
    queries.stateFor('t1').openTransactionOn = 'c1'
    const failing = queries.execute('t1', 'c1', 'UPDATE t SET a = 1')
    await Promise.resolve()
    again.release({ category: 'database', message: 'Bad.', detail: null })
    await failing
    expect(queries.stateFor('t1').openTransactionOn).toBe('c1')
  })

  it('forgets an open transaction when the tab or the connection lets go of the session', async () => {
    const queries = useQueryStore()
    const connections = useConnectionsStore()
    connections.active = { c1: infoFixture('c1'), c2: infoFixture('c2') }
    queries.stateFor('t1').openTransactionOn = 'c1'
    queries.stateFor('t2').openTransactionOn = 'c2'
    queries.stateFor('t3')

    queries.forgetTransaction('t1')
    queries.forgetTransaction('none')
    expect(queries.stateFor('t1').openTransactionOn).toBeNull()

    delete connections.active.c2
    await Promise.resolve()
    expect(queries.stateFor('t2').openTransactionOn).toBeNull()
  })

  it('refuses an empty statement', async () => {
    const queries = useQueryStore()
    expect(await queries.execute('t1', 'c1', '   ')).toBe(false)
    expect(apiStub.executeQuery).not.toHaveBeenCalled()
    expect(useUiStore().notices[0]?.message).toBe('There is nothing to run.')
  })

  it('sends the row limit of the connection when it is the smaller one', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response()))
    const fixture = connectionFixture()
    fixture.options.maxRows = 7
    apiStub.getConnections.mockResolvedValue([fixture])
    await useConnectionsStore().load()

    await useQueryStore().execute('t1', 'c1', 'SELECT 1')
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ options: { maxRows: 7, timeoutSecs: 300 } }),
      expect.anything(),
    )
  })

  it('sends the statement to the backend and keeps the result', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response()))
    const settings = useSettingsStore()
    settings.update({ maxRows: 25 })
    const connections = useConnectionsStore()
    await connections.load()

    const queries = useQueryStore()
    expect(await queries.execute('t1', 'c1', ' SELECT 1 ')).toBe(true)

    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      {
        connectionId: 'c1',
        requestId: expect.any(String),
        query: 'SELECT 1',
        tabId: 't1',
        queryParams: undefined,
        options: { maxRows: 25, timeoutSecs: 300 },
        pauseSecs: 0,
      },
      // The second argument holds the handlers that read the rows.
      expect.anything(),
    )

    const state = queries.stateFor('t1')
    expect(state.panes).toHaveLength(1)
    expect(state.panes[0]?.number).toBe(1)
    expect(state.activePaneId).toBe(state.panes[0]?.id)
    expect(state.messages).toEqual([{ level: 'info', text: '1 row returned.', detail: null }])
    expect(state.elapsedMs).toBe(12)
    expect(state.running).toBe(false)
    expect(state.requestId).toBeNull()
  })

  it('shows a set while it streams and grows the count of its rows', async () => {
    const counts: Array<{ panes: number; rows: number | undefined }> = []
    apiStub.executeQuery.mockImplementation(async (_request, handlers) => {
      const table = new ResultTable([{ name: 'n', typeName: 'int' }])
      const state = useQueryStore().stateFor('t1')
      const note = () => counts.push({ panes: state.panes.length, rows: state.panes[0]?.rows })
      handlers.onBegin?.(table)
      note()
      table.addSegment([], 2)
      handlers.onRows?.(table)
      note()
      table.addSegment([], 1)
      handlers.onRows?.(table)
      note()
      handlers.onSet(table)
      handlers.onEnd({ messages: [], rowsAffected: null, elapsedMs: 1, stats: null })
    })
    const connections = useConnectionsStore()
    await connections.load()

    const queries = useQueryStore()
    expect(await queries.execute('t1', 'c1', 'SELECT 1')).toBe(true)

    // The pane opened with the set and the count followed the rows. The end
    // of the set opened no second pane.
    expect(counts).toEqual([
      { panes: 1, rows: 0 },
      { panes: 1, rows: 2 },
      { panes: 1, rows: 3 },
    ])
    const state = queries.stateFor('t1')
    expect(state.panes).toHaveLength(1)
    expect(state.panes[0]?.rows).toBe(3)
  })

  it('copies the mark of the row limit to the pane when the set ends', async () => {
    const marks: boolean[] = []
    apiStub.executeQuery.mockImplementation(async (_request, handlers) => {
      const table = new ResultTable([{ name: 'n', typeName: 'int' }])
      const state = useQueryStore().stateFor('t1')
      handlers.onBegin?.(table)
      marks.push(state.panes[0]!.truncated)
      table.truncated = true
      handlers.onSet(table)
      marks.push(state.panes[0]!.truncated)
      handlers.onEnd({ messages: [], rowsAffected: null, elapsedMs: 1, stats: null })
    })
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(marks).toEqual([false, true])
  })

  it('shows the progress of a saved set until the set ends', async () => {
    const seen: unknown[] = []
    apiStub.executeQuery.mockImplementation(async (_request, handlers) => {
      const state = useQueryStore().stateFor('t1')
      const table = new ResultTable([{ name: 'n', typeName: 'int' }])
      handlers.onBegin?.(table)
      handlers.onProgress?.({ set: 0, rows: 50_000, bytes: 4096 })
      seen.push(state.saving)
      handlers.onSet(table)
      seen.push(state.saving)
      handlers.onProgress?.({ set: 1, rows: 7, bytes: 8 })
      handlers.onEnd({ messages: [], rowsAffected: null, elapsedMs: 1, stats: null })
    })
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(seen).toEqual([{ rows: 50_000, bytes: 4096, stopping: false }, null])
    // The end of the run clears the progress of a set that never ended.
    expect(queries.stateFor('t1').saving).toBeNull()
  })

  it('stops saving the rows of a running query once', async () => {
    const queries = useQueryStore()
    const ui = useUiStore()
    const warn = vi.spyOn(ui, 'warn')
    const steps: unknown[] = []
    // A tab with no request has nothing to stop.
    queries.stateFor('t1').saving = { rows: 1, bytes: 1, stopping: false }
    await queries.stopSaving('t1')
    expect(apiStub.stopSaving).not.toHaveBeenCalled()
    apiStub.executeQuery.mockImplementation(async (_request, handlers) => {
      const state = queries.stateFor('t1')
      handlers.onBegin?.(new ResultTable([{ name: 'n', typeName: 'int' }]))
      // Nothing is saving yet, so a stop does nothing.
      await queries.stopSaving('t1')
      handlers.onProgress?.({ set: 0, rows: 1, bytes: 2 })
      await queries.stopSaving('t1')
      await queries.stopSaving('t1')
      // A later report keeps the mark of the stop.
      handlers.onProgress?.({ set: 0, rows: 3, bytes: 4 })
      steps.push({ ...state.saving })
      // A stop that fails lets the user try again.
      apiStub.stopSaving.mockRejectedValueOnce(new Error('gone'))
      state.saving!.stopping = false
      await queries.stopSaving('t1')
      steps.push(state.saving!.stopping)
      handlers.onEnd({ messages: [], rowsAffected: null, elapsedMs: 1, stats: null })
    })
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(apiStub.stopSaving).toHaveBeenCalledTimes(2)
    expect(steps).toEqual([{ rows: 3, bytes: 4, stopping: true }, false])
    expect(warn).toHaveBeenCalledWith("Couldn't stop saving the rows.", 'gone')
  })

  it('passes over rows for a set that it never opened', async () => {
    apiStub.executeQuery.mockImplementation(async (_request, handlers) => {
      handlers.onRows?.(ResultTable.fromRows([], [[1]]))
      handlers.onEnd({ messages: [], rowsAffected: null, elapsedMs: 1, stats: null })
    })
    const queries = useQueryStore()
    expect(await queries.execute('t1', 'c1', 'SELECT 1')).toBe(true)
    expect(queries.stateFor('t1').panes).toEqual([])
  })

  it('uses the default time limit for a connection it does not know', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response()))
    const queries = useQueryStore()
    await queries.execute('t1', 'unknown', 'SELECT 1')
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ options: { maxRows: 10000, timeoutSecs: 300 } }),
      expect.anything(),
    )
  })

  it('warns when the row limit stopped the read', async () => {
    apiStub.executeQuery.mockImplementation(
      streamed({ ...response(), results: [{ columns: [], rows: [[1]], truncated: true }] }),
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(useUiStore().notices.some((notice) => notice.level === 'warning')).toBe(true)
  })

  it('refuses a second statement while the first one runs', async () => {
    let release: (value: unknown) => void = () => {}
    apiStub.executeQuery.mockReturnValue(
      new Promise((resolve) => {
        release = resolve
      }),
    )
    const queries = useQueryStore()
    const first = queries.execute('t1', 'c1', 'SELECT 1')
    expect(await queries.execute('t1', 'c1', 'SELECT 2')).toBe(false)
    release(undefined)
    await first
  })

  it('keeps the reason a statement failed', async () => {
    apiStub.executeQuery.mockRejectedValue({
      category: 'database',
      message: 'no such column',
      detail: null,
    })
    const queries = useQueryStore()
    expect(await queries.execute('t1', 'c1', 'SELECT bad')).toBe(false)
    expect(queries.stateFor('t1').error?.message).toBe('no such column')
    expect(queries.stateFor('t1').running).toBe(false)
  })

  it('marks a failed run, shows the messages and gives the place in the editor', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    expect(state.activePaneId).not.toBeNull()

    apiStub.executeQuery.mockRejectedValue({
      category: 'database',
      message: 'no such column',
      detail: null,
      line: 2,
      column: 5,
    })
    await queries.execute('t1', 'c1', 'SELECT 1,\n    bad', undefined, { line: 10, column: 1 })
    expect(state.failed).toBe(true)
    expect(state.activePaneId).toBeNull()
    expect(state.errorLocation).toEqual({ line: 11, column: 5 })

    // A failure with a line and no column points at the start of the line.
    const lineOnly = { category: 'database', message: 'bad', detail: null, line: 1 }
    apiStub.executeQuery.mockRejectedValue(lineOnly)
    await queries.execute('t1', 'c1', 'SELECT bad', undefined, { line: 3, column: 7 })
    expect(state.errorLocation).toEqual({ line: 3, column: 7 })

    // A plan failure gives no place, because the backend counts in the text
    // with the plan keyword in front.
    apiStub.explainQuery.mockRejectedValue(lineOnly)
    await queries.explain('t1', 'c1', 'SELECT bad', 'estimated')
    expect(state.failed).toBe(true)
    expect(state.errorLocation).toBeNull()

    // An edit in the editor removes the place, and a tab without state is
    // left alone.
    await queries.execute('t1', 'c1', 'SELECT bad', undefined, { line: 3, column: 7 })
    queries.clearErrorLocation('t1')
    expect(state.errorLocation).toBeNull()
    queries.clearErrorLocation('missing')
    expect(queries.peekState('missing')).toBeUndefined()

    // A failure with no line, or a run with no start, gives no place.
    await queries.execute('t1', 'c1', 'SELECT bad')
    expect(state.errorLocation).toBeNull()
    apiStub.executeQuery.mockRejectedValue({ category: 'database', message: 'x', detail: null })
    await queries.execute('t1', 'c1', 'SELECT bad', undefined, { line: 1, column: 1 })
    expect(state.errorLocation).toBeNull()

    // The next run clears the mark of the failure.
    apiStub.executeQuery.mockImplementation(streamed(response()))
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(state.failed).toBe(false)
  })

  it('adds each message as it streams, then the messages of the end', async () => {
    apiStub.executeQuery.mockImplementation(
      async (_request: unknown, handlers: import('@/lib/results').ResultStreamHandlers) => {
        handlers.onMessage?.({ level: 'info', text: 'first', detail: null })
        expect(
          useQueryStore()
            .stateFor('t1')
            .messages.map((m) => m.text),
        ).toEqual(['first'])
        handlers.onEnd({
          messages: [{ level: 'info', text: 'last', detail: null }],
          rowsAffected: null,
          elapsedMs: 1,
          stats: null,
        })
      },
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(queries.stateFor('t1').messages.map((m) => m.text)).toEqual(['first', 'last'])
  })

  it('adds each message to the same list, and copies no list', async () => {
    let list: unknown = null
    apiStub.executeQuery.mockImplementation(
      async (_request: unknown, handlers: import('@/lib/results').ResultStreamHandlers) => {
        list = useQueryStore().stateFor('t1').messages
        for (let index = 0; index < 3; index += 1) {
          handlers.onMessage?.({ level: 'info', text: `line ${index}`, detail: null })
        }
        handlers.onEnd({
          messages: [{ level: 'info', text: 'last', detail: null }],
          rowsAffected: null,
          elapsedMs: 1,
          stats: null,
        })
      },
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(queries.stateFor('t1').messages).toBe(list)
    expect(queries.stateFor('t1').messages).toHaveLength(4)
  })

  it('keeps the last messages of a long run and counts the others', async () => {
    const total = 2 * KEPT_MESSAGES + 10
    apiStub.executeQuery.mockImplementation(
      async (_request: unknown, handlers: import('@/lib/results').ResultStreamHandlers) => {
        for (let index = 0; index < total - 1; index += 1) {
          handlers.onMessage?.({ level: 'info', text: `line ${index}`, detail: null })
        }
        handlers.onEnd({
          messages: [{ level: 'info', text: 'last', detail: null }],
          rowsAffected: null,
          elapsedMs: 1,
          stats: null,
        })
      },
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    expect(state.messages).toHaveLength(KEPT_MESSAGES + 10)
    expect(state.droppedMessages).toBe(KEPT_MESSAGES)
    expect(state.messages[0]?.text).toBe(`line ${KEPT_MESSAGES}`)
    expect(state.messages[state.messages.length - 1]?.text).toBe('last')

    // The next run starts a new count.
    apiStub.executeQuery.mockImplementation(
      async (_request: unknown, handlers: import('@/lib/results').ResultStreamHandlers) => {
        handlers.onEnd({ messages: [], rowsAffected: null, elapsedMs: 1, stats: null })
      },
    )
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(state.messages).toHaveLength(0)
    expect(state.droppedMessages).toBe(0)
  })

  it('reports a length of time even when the start is no longer known', async () => {
    apiStub.executeQuery.mockImplementation(async () => {
      useQueryStore().stateFor('t1').startedAt = null
      throw { category: 'database', message: 'no', detail: null }
    })
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(queries.stateFor('t1').elapsedMs).toBe(0)
  })

  it('writes every execution to the history', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response()))
    const connections = useConnectionsStore()
    await connections.load()
    const history = useHistoryStore()
    const record = vi.spyOn(history, 'record')

    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(record).toHaveBeenCalledWith({
      connectionId: 'c1',
      connectionName: 'Server',
      query: 'SELECT 1',
      elapsedMs: 12,
      rowCount: 1,
      succeeded: true,
      error: null,
    })
  })

  it('reports that the record of a connection is gone in the history', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response()))
    const history = useHistoryStore()
    const record = vi.spyOn(history, 'record')
    const queries = useQueryStore()
    await queries.execute('t1', 'lost', 'SELECT 1')
    expect(record).toHaveBeenCalledWith(
      expect.objectContaining({ connectionName: 'Deleted connection' }),
    )
  })

  it('reads the estimated plan and names the result', async () => {
    apiStub.explainQuery.mockResolvedValue(response())
    const connections = useConnectionsStore()
    await connections.load()
    const queries = useQueryStore()

    expect(await queries.explain('t1', 'c1', ' SELECT 1 ', 'estimated')).toBe(true)
    expect(apiStub.explainQuery).toHaveBeenCalledWith({
      connectionId: 'c1',
      requestId: expect.any(String),
      query: 'SELECT 1',
      mode: 'estimated',
      tabId: 't1',
      queryParams: undefined,
      options: { maxRows: 10000, timeoutSecs: 300 },
    })
    expect(queries.stateFor('t1').panes[0]?.label).toBe('Estimated plan')
    // A plan is not the statement of the user, so the history holds none.
    expect(apiStub.addHistoryEntry).not.toHaveBeenCalled()
  })

  it('names the result of an actual plan', async () => {
    apiStub.explainQuery.mockResolvedValue(response())
    const queries = useQueryStore()
    await queries.explain('t1', 'c1', 'SELECT 1', 'actual')
    expect(queries.stateFor('t1').panes[0]?.label).toBe('Actual plan')
    expect(apiStub.explainQuery).toHaveBeenCalledWith(expect.objectContaining({ mode: 'actual' }))
  })

  it('refuses a plan of an empty statement', async () => {
    const queries = useQueryStore()
    expect(await queries.explain('t1', 'c1', '  ', 'estimated')).toBe(false)
    expect(apiStub.explainQuery).not.toHaveBeenCalled()
  })

  it('asks the backend to stop a statement that runs', async () => {
    let release: (value: unknown) => void = () => {}
    apiStub.executeQuery.mockReturnValue(
      new Promise((resolve) => {
        release = resolve
      }),
    )
    apiStub.cancelQuery.mockResolvedValue(undefined)
    const queries = useQueryStore()
    const running = queries.execute('t1', 'c1', 'SELECT 1')
    const requestId = queries.stateFor('t1').requestId

    await queries.cancel('t1')
    expect(apiStub.cancelQuery).toHaveBeenCalledWith('c1', requestId)
    expect(queries.stateFor('t1').stopping).toBe(true)
    // A second press of Stop sends nothing more.
    await queries.cancel('t1')
    expect(apiStub.cancelQuery).toHaveBeenCalledTimes(1)
    release(undefined)
    await running
    expect(queries.stateFor('t1').stopping).toBe(false)
  })

  it('keeps with each result the statement, the values and the connection of its run', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response()))
    apiStub.explainQuery.mockResolvedValue(response())
    const queries = useQueryStore()

    await queries.execute('t1', 'c1', ' SELECT :id ', { id: 7 })
    expect(queries.stateFor('t1').panes[0]?.run).toEqual({
      connectionId: 'c1',
      query: 'SELECT :id',
      params: { id: 7 },
    })

    // A plan is not a run, so its result has no statement to run again.
    await queries.explain('t1', 'c1', 'SELECT 2', 'estimated')
    expect(queries.stateFor('t1').panes[0]?.run).toBeNull()
  })

  it('sends the stop to the connection of the run', async () => {
    let release: (value: unknown) => void = () => {}
    apiStub.executeQuery.mockReturnValue(
      new Promise((resolve) => {
        release = resolve
      }),
    )
    apiStub.cancelQuery.mockResolvedValue(undefined)
    const queries = useQueryStore()
    const running = queries.execute('t1', 'old-connection', 'SELECT 1')
    expect(queries.stateFor('t1').requestConnectionId).toBe('old-connection')

    await queries.cancel('t1')
    expect(apiStub.cancelQuery).toHaveBeenCalledWith(
      'old-connection',
      queries.stateFor('t1').requestId,
    )
    release(undefined)
    await running
    expect(queries.stateFor('t1').requestConnectionId).toBeNull()
  })

  it('does nothing when there is no statement to stop', async () => {
    const queries = useQueryStore()
    await queries.cancel('t1')
    expect(apiStub.cancelQuery).not.toHaveBeenCalled()
  })

  it('notes a failure to stop a statement', async () => {
    let release: (value: unknown) => void = () => {}
    apiStub.executeQuery.mockReturnValue(
      new Promise((resolve) => {
        release = resolve
      }),
    )
    apiStub.cancelQuery.mockRejectedValue(new Error('the server refused'))
    const queries = useQueryStore()
    const running = queries.execute('t1', 'c1', 'SELECT 1')
    await queries.cancel('t1')
    const notice = useUiStore().notices.find((item) => item.level === 'warning')
    expect(notice?.message).toBe("Couldn't stop the statement. It may still be running.")
    expect(notice?.detail).toBe('the server refused')
    expect(queries.stateFor('t1').stopping).toBe(false)
    release(undefined)
    await running
  })

  it('moves to a result that is there, and to the messages', async () => {
    apiStub.executeQuery.mockImplementation(streamed(twoResults()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    const first = state.panes[0]!.id

    queries.selectPane('t1', first)
    expect(state.activePaneId).toBe(first)

    queries.selectPane('t1', 'no-such-result')
    expect(state.activePaneId).toBe(first)

    queries.selectPane('t1', null)
    expect(state.activePaneId).toBeNull()
  })

  it('keeps a result against the next run and lets it go again', async () => {
    apiStub.executeQuery.mockImplementation(streamed(twoResults()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    const kept = state.panes[0]!.id

    queries.togglePin('t1', kept)
    expect(state.panes.find((pane) => pane.id === kept)?.pinned).toBe(true)

    await queries.execute('t1', 'c1', 'SELECT 2')
    // The kept result stays, and the run added two more.
    expect(state.panes).toHaveLength(3)
    expect(state.panes[0]?.id).toBe(kept)

    queries.togglePin('t1', kept)
    await queries.execute('t1', 'c1', 'SELECT 3')
    expect(state.panes).toHaveLength(2)
  })

  it('numbers the results of the run and not of the list', async () => {
    apiStub.executeQuery.mockImplementation(streamed(twoResults()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    queries.togglePin('t1', state.panes[0]!.id)
    await queries.execute('t1', 'c1', 'SELECT 2')
    expect(state.panes.map((pane) => pane.number)).toEqual([1, 1, 2])
  })

  it('refuses to keep more results than the settings allow', async () => {
    apiStub.executeQuery.mockImplementation(streamed(twoResults()))
    const queries = useQueryStore()
    useSettingsStore().update({ maxPinnedResults: 1 })
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')

    queries.togglePin('t1', state.panes[0]!.id)
    queries.togglePin('t1', state.panes[1]!.id)
    expect(state.panes[1]?.pinned).toBe(false)
    expect(useUiStore().notices.some((notice) => notice.level === 'warning')).toBe(true)
  })

  it('keeps nothing for a result that is not there', async () => {
    apiStub.executeQuery.mockImplementation(streamed(twoResults()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    queries.togglePin('t1', 'no-such-result')
    expect(queries.stateFor('t1').panes.every((pane) => !pane.pinned)).toBe(true)
  })

  it('closes a result and shows the one that takes its place', async () => {
    apiStub.executeQuery.mockImplementation(streamed(twoResults()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    const [first, second] = [state.panes[0]!.id, state.panes[1]!.id]

    queries.selectPane('t1', first)
    queries.closePane('t1', first)
    expect(state.panes).toHaveLength(1)
    expect(state.activePaneId).toBe(second)

    queries.closePane('t1', second)
    expect(state.activePaneId).toBeNull()
  })

  it('closes the last result and steps back to the one before it', async () => {
    apiStub.executeQuery.mockImplementation(streamed(twoResults()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    const last = state.panes[1]!.id
    queries.closePane('t1', last)
    expect(state.activePaneId).toBe(state.panes[0]?.id)
  })

  it('keeps the result on show when another one closes', async () => {
    apiStub.executeQuery.mockImplementation(streamed(twoResults()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    const second = state.panes[1]!.id

    queries.selectPane('t1', second)
    queries.closePane('t1', state.panes[0]!.id)
    expect(state.activePaneId).toBe(second)
  })

  it('records the scan of a statement and adds it to the session', async () => {
    apiStub.executeQuery.mockImplementation(
      streamed({
        ...response(),
        stats: { scannedBytes: 1024 ** 3, engineMs: 120, queueMs: 3, resultReused: false },
      }),
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    await queries.execute('t2', 'c1', 'SELECT 2')

    expect(queries.stateFor('t1').stats?.scannedBytes).toBe(1024 ** 3)
    expect(queries.sessionScannedBytes).toBe(2 * 1024 ** 3)
  })

  it('warns about a scan above the limit of the settings', async () => {
    useSettingsStore().update({ athenaScanWarningGb: 1, athenaPricePerTerabyte: 5 })
    apiStub.executeQuery.mockImplementation(
      streamed({
        ...response(),
        stats: { scannedBytes: 2 * 1024 ** 3, engineMs: null, queueMs: null, resultReused: null },
      }),
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')

    const notice = useUiStore().notices.find((item) => item.level === 'warning')
    expect(notice?.message).toContain('than the 1 GB warning limit')
    expect(notice?.detail).toContain('$0.01')
  })

  it('counts nothing for an engine that reports no scan', async () => {
    apiStub.executeQuery.mockImplementation(streamed(response()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(queries.stateFor('t1').stats).toBeNull()
    expect(queries.sessionScannedBytes).toBe(0)
  })

  it('closes nothing for a result that is not there', async () => {
    apiStub.executeQuery.mockImplementation(streamed(twoResults()))
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    queries.closePane('t1', 'no-such-result')
    expect(queries.stateFor('t1').panes).toHaveLength(2)
  })
})

describe('query store counting what runs on a connection', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
  })

  it('counts none while nothing runs', () => {
    expect(useQueryStore().runningOn('c1')).toBe(0)
  })

  it('counts the statements that run against one connection alone', () => {
    const queries = useQueryStore()
    const first = queries.stateFor('t1')
    first.running = true
    first.requestConnectionId = 'c1'
    const second = queries.stateFor('t2')
    second.running = true
    second.requestConnectionId = 'c1'
    const other = queries.stateFor('t3')
    other.running = true
    other.requestConnectionId = 'c2'

    expect(queries.runningOn('c1')).toBe(2)
    expect(queries.runningOn('c2')).toBe(1)
  })

  it('leaves out a statement that has finished', () => {
    const queries = useQueryStore()
    const state = queries.stateFor('t1')
    state.running = false
    state.requestConnectionId = 'c1'

    expect(queries.runningOn('c1')).toBe(0)
  })
})

describe('a run to a file', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.addHistoryEntry.mockResolvedValue([])
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([])
  })

  const summary = {
    rows: 50,
    truncated: false,
    path: '/a/out.csv',
    sheetFull: false,
    cutCells: 0,
    warning: null,
    sets: [{ path: '/a/out.csv', sheet: null, rows: 50, truncated: false, sheetFull: false }],
    skippedSets: 0,
  }

  /** A run whose first set, cut at the grid limit, also went to a file. */
  function savedRun(results = [{ columns: [], rows: [[1]], truncated: true }]) {
    const stream = streamed({ ...response(), results })
    apiStub.runToFile.mockImplementation(async (request: unknown, handlers: never) => {
      await stream(request, handlers)
      return summary
    })
  }

  it('sends the ticket and the export limit, and records the file on the first result', async () => {
    savedRun()
    const queries = useQueryStore()
    const result = await queries.runToFile(
      't1',
      'c1',
      ' SELECT 1 ',
      { ticket: 'k1', eachSet: false },
      { a: 1 },
      {
        line: 1,
        column: 1,
      },
    )
    expect(result).toEqual(summary)
    expect(apiStub.runToFile).toHaveBeenCalledWith(
      expect.objectContaining({
        connectionId: 'c1',
        query: 'SELECT 1',
        ticket: 'k1',
        maxRows: useSettingsStore().settings.exportRowLimit,
        eachSet: false,
        tabId: 't1',
        queryParams: { a: 1 },
        options: { maxRows: 10000, timeoutSecs: 300 },
      }),
      expect.anything(),
    )
    const pane = queries.stateFor('t1').panes[0]!
    expect(pane.savedFile).toEqual({ path: '/a/out.csv', rows: 50, truncated: false })
    // The cut of the first set is the preview, so no warning comes.
    expect(useUiStore().notices.some((notice) => notice.level === 'warning')).toBe(false)
  })

  it('warns when the row limit cut a set after the first', async () => {
    savedRun([
      { columns: [], rows: [[1]], truncated: true },
      { columns: [], rows: [[2]], truncated: true },
    ])
    const queries = useQueryStore()
    await queries.runToFile('t1', 'c1', 'SELECT 1; SELECT 2', {
      ticket: 'k1',
      eachSet: false,
    })
    expect(queries.stateFor('t1').panes[1]!.savedFile).toBeUndefined()
    expect(useUiStore().notices.some((notice) => notice.level === 'warning')).toBe(true)
  })

  it('records the file of each saved set and warns for no cut of a saved set', async () => {
    const stream = streamed({
      ...response(),
      results: [
        { columns: [], rows: [[1]], truncated: true },
        { columns: [], rows: [[2]], truncated: true },
        { columns: [], rows: [[3]], truncated: false },
      ],
    })
    const sheets = {
      ...summary,
      path: '/a/out.xlsx',
      sets: [
        { path: '/a/out.xlsx', sheet: 'Result 1', rows: 9, truncated: false, sheetFull: false },
        { path: '/a/out.xlsx', sheet: 'Result 2', rows: 8, truncated: true, sheetFull: false },
      ],
    }
    apiStub.runToFile.mockImplementation(async (request: unknown, handlers: never) => {
      await stream(request, handlers)
      return sheets
    })
    const queries = useQueryStore()
    await queries.runToFile('t1', 'c1', 'SELECT 1; SELECT 2', { ticket: 'k1', eachSet: true })
    expect(apiStub.runToFile).toHaveBeenCalledWith(
      expect.objectContaining({ eachSet: true }),
      expect.anything(),
    )
    const panes = queries.stateFor('t1').panes
    expect(panes[1]!.savedFile).toEqual({
      path: '/a/out.xlsx',
      rows: 8,
      truncated: true,
      sheet: 'Result 2',
    })
    // The run gave a third set that no file took, and no pane is cut there.
    expect(panes[2]!.savedFile).toBeUndefined()
    expect(useUiStore().notices.some((notice) => notice.level === 'warning')).toBe(false)
  })

  it('gives null for a run that failed', async () => {
    apiStub.runToFile.mockRejectedValue({ category: 'query', message: 'bad', detail: null })
    const queries = useQueryStore()
    expect(
      await queries.runToFile('t1', 'c1', 'SELECT 1', { ticket: 'k1', eachSet: false }),
    ).toBeNull()
    expect(queries.stateFor('t1').failed).toBe(true)
  })

  it('records nothing for a tab that closed during the run', async () => {
    const queries = useQueryStore()
    apiStub.runToFile.mockImplementation(async () => {
      queries.clear('t1')
      return summary
    })
    expect(
      await queries.runToFile('t1', 'c1', 'SELECT 1', { ticket: 'k1', eachSet: false }),
    ).toEqual(summary)
    expect(queries.peekState('t1')).toBeUndefined()
  })
})

describe('the file of the messages of a tab', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.addHistoryEntry.mockResolvedValue([])
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.forgetMessagesFile.mockResolvedValue(undefined)
    apiStub.saveRunMessages.mockResolvedValue(true)
  })

  const first = { id: 'f1', path: '/logs/first.txt' }
  const second = { id: 'f2', path: '/logs/second.txt' }

  it('arms a file that each later run of the tab names', async () => {
    const queries = useQueryStore()
    queries.stateFor('t1')
    apiStub.chooseMessagesFile.mockResolvedValue(first)
    await queries.saveAllMessages('t1')
    expect(apiStub.chooseMessagesFile).toHaveBeenCalledWith('messages.txt')
    expect(queries.stateFor('t1').messagesFile).toEqual(first)
    expect(apiStub.saveRunMessages).not.toHaveBeenCalled()

    apiStub.executeQuery.mockImplementation(streamed(response()))
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ messagesFile: 'f1' }),
      expect.anything(),
    )
    apiStub.runToFile.mockResolvedValue(null)
    await queries.runToFile('t1', 'c1', 'SELECT 1', { ticket: 'k1', eachSet: false })
    expect(apiStub.runToFile).toHaveBeenCalledWith(
      expect.objectContaining({ messagesFile: 'f1' }),
      expect.anything(),
    )
    // The file stays armed after the runs.
    expect(queries.stateFor('t1').messagesFile).toEqual(first)
  })

  it('sends a running run to the new file and forgets the old file', async () => {
    const queries = useQueryStore()
    const state = queries.stateFor('t1')
    state.messagesFile = first
    state.running = true
    state.requestId = 'r9'
    apiStub.chooseMessagesFile.mockResolvedValue(second)
    await queries.saveAllMessages('t1')
    expect(apiStub.saveRunMessages).toHaveBeenCalledWith('r9', 'f2')
    expect(apiStub.forgetMessagesFile).toHaveBeenCalledWith('f1')
    expect(state.messagesFile).toEqual(second)
  })

  it('does nothing when the user closes the dialog', async () => {
    const queries = useQueryStore()
    queries.stateFor('t1')
    apiStub.chooseMessagesFile.mockResolvedValue(null)
    await queries.saveAllMessages('t1')
    expect(queries.stateFor('t1').messagesFile).toBeNull()
  })

  it('forgets a file that the user chose after the tab closed', async () => {
    const queries = useQueryStore()
    apiStub.chooseMessagesFile.mockResolvedValue(first)
    await queries.saveAllMessages('closed')
    expect(apiStub.forgetMessagesFile).toHaveBeenCalledWith('f1')
    expect(queries.peekState('closed')).toBeUndefined()
  })

  it('reports a failure of the choice', async () => {
    const queries = useQueryStore()
    apiStub.chooseMessagesFile.mockRejectedValue({
      category: 'io',
      message: "Couldn't create /x.txt",
      detail: null,
    })
    await queries.saveAllMessages('t1')
    expect(useUiStore().notices.some((notice) => notice.message.includes('/x.txt'))).toBe(true)
  })

  it('stops the writes when the user asks and when the tab closes', async () => {
    const queries = useQueryStore()
    queries.stateFor('t1').messagesFile = first
    queries.stopSavingMessages('t1')
    expect(apiStub.forgetMessagesFile).toHaveBeenCalledWith('f1')
    expect(queries.stateFor('t1').messagesFile).toBeNull()
    // A tab without a file has nothing to stop.
    queries.stopSavingMessages('t1')
    queries.stopSavingMessages('none')
    expect(apiStub.forgetMessagesFile).toHaveBeenCalledTimes(1)

    // A failure of the backend leaves the tab without the file.
    apiStub.forgetMessagesFile.mockRejectedValue(new Error('gone'))
    queries.stateFor('t1').messagesFile = second
    queries.clear('t1')
    await Promise.resolve()
    expect(apiStub.forgetMessagesFile).toHaveBeenLastCalledWith('f2')
    expect(queries.peekState('t1')).toBeUndefined()
  })

  it('saves the messages that the tab shows', async () => {
    const queries = useQueryStore()
    const state = queries.stateFor('t1')
    state.messages = [{ level: 'info', text: 'a', detail: null }]
    state.droppedMessages = 4
    apiStub.saveShownMessages.mockResolvedValue('/logs/shown.txt')
    await queries.saveShownMessages('t1')
    expect(apiStub.saveShownMessages).toHaveBeenCalledWith({
      defaultName: 'messages.txt',
      messages: [{ level: 'info', text: 'a', detail: null }],
      dropped: 4,
    })
    const ui = useUiStore()
    expect(ui.notices.some((notice) => notice.message.includes('/logs/shown.txt'))).toBe(true)

    // A closed dialog gives no notice, and a failure gives an error.
    const count = ui.notices.length
    apiStub.saveShownMessages.mockResolvedValue(null)
    await queries.saveShownMessages('t1')
    expect(ui.notices.length).toBe(count)
    apiStub.saveShownMessages.mockRejectedValue(new Error('disk full'))
    await queries.saveShownMessages('t1')
    expect(ui.notices.some((notice) => notice.message.includes('disk full'))).toBe(true)
  })
})

describe('kept results', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.addHistoryEntry.mockResolvedValue([])
    apiStub.releaseKept.mockResolvedValue(undefined)
  })

  /** Lets the releases that the store does not await reach the stub. */
  const released = async (): Promise<string[]> => {
    await new Promise((resolve) => setTimeout(resolve, 0))
    return apiStub.releaseKept.mock.calls.map((call) => call[0] as string)
  }

  /** A run of two cut sets whose full rows the backend kept. */
  function keptRun(request: string) {
    apiStub.executeQuery.mockImplementationOnce(
      streamed({
        results: [
          { rows: [[1]], truncated: true },
          { rows: [[2]], truncated: true },
        ],
        kept: [
          { set: 0, id: `${request}:0`, origin: 'athena', keptAt: 5 },
          { set: 1, id: `${request}:1`, origin: 'athena', keptAt: 5 },
        ],
      }),
    )
  }

  it('gives each kept set to the pane of its table', async () => {
    keptRun('r1')
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const panes = queries.stateFor('t1').panes
    expect(panes.map((pane) => pane.keptId)).toEqual(['r1:0', 'r1:1'])
    expect(await released()).toEqual([])
  })

  it('releases the kept results that a new run drops, and keeps a pinned one', async () => {
    keptRun('r1')
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    queries.togglePin('t1', queries.stateFor('t1').panes[1]!.id)

    apiStub.executeQuery.mockImplementationOnce(streamed({ results: [{ rows: [[3]] }] }))
    await queries.execute('t1', 'c1', 'SELECT 2')
    expect(await released()).toEqual(['r1:0'])
    expect(queries.stateFor('t1').panes[0]!.keptId).toBe('r1:1')
  })

  it('releases the kept result of a closed pane and of a closed tab', async () => {
    keptRun('r1')
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    queries.closePane('t1', queries.stateFor('t1').panes[0]!.id)
    expect(await released()).toEqual(['r1:0'])

    queries.clear('t1')
    expect(await released()).toEqual(['r1:0', 'r1:1'])
  })

  it('releases a kept set whose pane is gone before the run ends', async () => {
    apiStub.executeQuery.mockImplementationOnce(
      async (_request: unknown, handlers: import('@/lib/results').ResultStreamHandlers) => {
        handlers.onSet(ResultTable.fromRows([], [[1]], true))
        useQueryStore().clear('t1')
        handlers.onEnd({
          messages: [],
          rowsAffected: null,
          elapsedMs: 1,
          stats: null,
          kept: [{ set: 0, id: 'r1:0', origin: 'athena', keptAt: 5 }],
        })
      },
    )
    await useQueryStore().execute('t1', 'c1', 'SELECT 1')
    expect(await released()).toEqual(['r1:0'])
  })

  it('asks the backend to save full results only when Settings turns it on', async () => {
    const queries = useQueryStore()
    const settings = useSettingsStore()
    apiStub.executeQuery.mockImplementation(streamed({ results: [{ rows: [[1]] }] }))
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(apiStub.executeQuery.mock.calls[0]![0]).toMatchObject({ spill: undefined })

    settings.update({ keepFullResults: true, exportRowLimit: 5000, fullResultsDiskGb: 3 })
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(apiStub.executeQuery.mock.calls[1]![0]).toMatchObject({
      spill: { maxRows: 5000, maxBytes: 3 * 1024 ** 3 },
    })
  })

  it('asks the backend to save full results for a run that keeps all rows', async () => {
    const queries = useQueryStore()
    useSettingsStore().update({
      keepFullResults: false,
      exportRowLimit: 5000,
      fullResultsDiskGb: 3,
    })
    apiStub.executeQuery.mockImplementation(streamed({ results: [{ rows: [[1]] }] }))
    await queries.execute('t1', 'c1', 'SELECT 1', undefined, undefined, true)
    expect(apiStub.executeQuery.mock.calls[0]![0]).toMatchObject({
      spill: { maxRows: 5000, maxBytes: 3 * 1024 ** 3 },
    })
  })

  it('asks the backend to pause a read only when Settings turns it on and nothing is saved', async () => {
    const queries = useQueryStore()
    const settings = useSettingsStore()
    apiStub.executeQuery.mockImplementation(streamed({ results: [{ rows: [[1]] }] }))
    const sent = async () => {
      await queries.execute('t1', 'c1', 'SELECT 1')
      return (
        apiStub.executeQuery.mock.calls[apiStub.executeQuery.mock.calls.length - 1]![0] as {
          pauseSecs: number
        }
      ).pauseSecs
    }
    expect(await sent()).toBe(0)
    settings.update({ pauseAtRowLimit: true, pauseLimitMinutes: 7 })
    expect(await sent()).toBe(420)
    // Saved full results give every row, so the read does not pause.
    settings.update({ keepFullResults: true })
    expect(await sent()).toBe(0)
  })

  /** A run whose first set paused at the row limit for `seconds`. */
  function pausedRun(request: string, seconds = 600) {
    apiStub.executeQuery.mockImplementationOnce(
      streamed({
        results: [{ rows: [[1]], truncated: true }],
        kept: [{ set: 0, id: `${request}:0`, origin: 'paused', keptAt: 0, pausedSecs: seconds }],
      }),
    )
  }

  it('marks a paused result with the end of its pause and gives no row limit warning', async () => {
    vi.useFakeTimers({ now: 1_000_000 })
    try {
      const queries = useQueryStore()
      const warn = vi.spyOn(useUiStore(), 'warn')
      pausedRun('r1', 60)
      await queries.execute('t1', 'c1', 'SELECT 1')
      const pane = queries.stateFor('t1').panes[0]!
      expect(pane).toMatchObject({ keptId: 'r1:0', pausedUntil: 1_060_000 })
      expect(warn).not.toHaveBeenCalled()

      // The end of the pause forgets the read, as the backend does.
      vi.advanceTimersByTime(60_000)
      expect(pane.keptId).toBeUndefined()
      expect(pane.kept).toBeUndefined()
      expect(pane.pausedUntil).toBeUndefined()
    } finally {
      vi.useRealTimers()
    }
  })

  it('leaves a pane alone when its pause ends after the pane moved on', async () => {
    vi.useFakeTimers()
    try {
      const queries = useQueryStore()
      pausedRun('r1', 60)
      await queries.execute('t1', 'c1', 'SELECT 1')
      const pane = queries.stateFor('t1').panes[0]!
      queries.endPause(pane)
      pane.keptId = 'other'
      vi.advanceTimersByTime(60_000)
      expect(pane.keptId).toBe('other')
    } finally {
      vi.useRealTimers()
    }
  })

  it('releases a paused read on request and on the next run, also when pinned', async () => {
    const queries = useQueryStore()
    pausedRun('r1')
    await queries.execute('t1', 'c1', 'SELECT 1')
    const first = queries.stateFor('t1').panes[0]!
    queries.endPause(first, true)
    expect(await released()).toEqual(['r1:0'])
    expect(first.pausedUntil).toBeUndefined()

    pausedRun('r2')
    await queries.execute('t1', 'c1', 'SELECT 1')
    const pinned = queries.stateFor('t1').panes[0]!
    queries.togglePin('t1', pinned.id)
    apiStub.executeQuery.mockImplementationOnce(streamed({ results: [{ rows: [[2]] }] }))
    await queries.execute('t1', 'c1', 'SELECT 2')
    expect(await released()).toEqual(['r1:0', 'r2:0'])
    expect(pinned).toMatchObject({ pinned: true, keptId: undefined, pausedUntil: undefined })
  })

  /** A paused run whose read names its session on the server. */
  function pausedSession(request: string, extra: Record<string, unknown> = {}) {
    apiStub.executeQuery.mockImplementationOnce(
      streamed({
        results: [{ rows: [[1]], truncated: true }],
        kept: [
          {
            set: 0,
            id: `${request}:0`,
            origin: 'paused',
            keptAt: 0,
            pausedSecs: 600,
            serverSession: 52,
            ...extra,
          },
        ],
      }),
    )
  }

  it('checks each minute which sessions a paused read blocks', async () => {
    vi.useFakeTimers()
    try {
      const queries = useQueryStore()
      pausedSession('r1', { pauseAtMost: true, serverIdleSecs: 300 })
      await queries.execute('t1', 'c1', 'SELECT 1')
      const pane = queries.stateFor('t1').panes[0]!
      expect(pane.pause).toEqual({
        connectionId: 'c1',
        serverSession: 52,
        blocking: [],
        atMost: true,
        serverIdleSecs: 300,
      })
      const sessions = [{ waitingSession: 57, blockingSession: 52 }]
      apiStub.blockingSessions.mockResolvedValueOnce({ sessions, openTransactions: [], notes: [] })
      await vi.advanceTimersByTimeAsync(60_000)
      expect(apiStub.blockingSessions).toHaveBeenCalledWith('c1', 52)
      expect(pane.pause!.blocking).toEqual(sessions)
      // A failed check keeps the last answer.
      apiStub.blockingSessions.mockRejectedValueOnce(new Error('busy'))
      await vi.advanceTimersByTimeAsync(60_000)
      expect(pane.pause!.blocking).toEqual(sessions)
      // The end of the pause stops the checks.
      queries.endPause(pane, true)
      expect(pane.pause).toBeUndefined()
      await vi.advanceTimersByTimeAsync(120_000)
      expect(apiStub.blockingSessions).toHaveBeenCalledTimes(2)
    } finally {
      vi.useRealTimers()
    }
  })

  it('does not check a paused read whose session is unknown', async () => {
    vi.useFakeTimers()
    try {
      const queries = useQueryStore()
      pausedRun('r1', 600)
      await queries.execute('t1', 'c1', 'SELECT 1')
      const pane = queries.stateFor('t1').panes[0]!
      expect(pane.pause).toMatchObject({ connectionId: 'c1', atMost: false })
      await vi.advanceTimersByTimeAsync(120_000)
      expect(apiStub.blockingSessions).not.toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })

  it('extends a paused read and moves the end of its pause', async () => {
    vi.useFakeTimers({ now: 1_000_000 })
    try {
      const queries = useQueryStore()
      apiStub.blockingSessions.mockResolvedValue({ sessions: [], openTransactions: [], notes: [] })
      pausedSession('r1')
      await queries.execute('t1', 'c1', 'SELECT 1')
      const pane = queries.stateFor('t1').panes[0]!
      apiStub.extendPause.mockResolvedValueOnce({ pausedSecs: 1200, atMost: true })
      await queries.extendPause(pane)
      expect(apiStub.extendPause).toHaveBeenCalledWith('r1:0')
      expect(pane.pausedUntil).toBe(Date.now() + 1_200_000)
      expect(pane.pause!.atMost).toBe(true)
      // The old end no longer forgets the read.
      await vi.advanceTimersByTimeAsync(600_000)
      expect(pane.keptId).toBe('r1:0')
      await vi.advanceTimersByTimeAsync(600_000)
      expect(pane.keptId).toBeUndefined()
      // A pane without a paused read asks for nothing.
      await queries.extendPause(pane)
      expect(apiStub.extendPause).toHaveBeenCalledTimes(1)
    } finally {
      vi.useRealTimers()
    }
  })

  it('forgets a paused read that the backend no longer has', async () => {
    const queries = useQueryStore()
    const report = vi.spyOn(useUiStore(), 'reportError')
    pausedSession('r1')
    await queries.execute('t1', 'c1', 'SELECT 1')
    const pane = queries.stateFor('t1').panes[0]!
    apiStub.extendPause.mockRejectedValueOnce(new Error('gone'))
    await queries.extendPause(pane)
    expect(report).toHaveBeenCalled()
    expect(pane.keptId).toBeUndefined()

    // An answer for a pane that moved on changes nothing.
    pausedSession('r2')
    await queries.execute('t1', 'c1', 'SELECT 1')
    const next = queries.stateFor('t1').panes[0]!
    let answer: (value: unknown) => void = () => undefined
    apiStub.extendPause.mockReturnValueOnce(new Promise((resolve) => (answer = resolve)))
    const extending = queries.extendPause(next)
    queries.endPause(next, true)
    answer({ pausedSecs: 1200, atMost: false })
    await extending
    expect(next.pause).toBeUndefined()
    let fail: (error: unknown) => void = () => undefined
    pausedSession('r3')
    await queries.execute('t1', 'c1', 'SELECT 1')
    const last = queries.stateFor('t1').panes[0]!
    apiStub.extendPause.mockReturnValueOnce(new Promise((_, reject) => (fail = reject)))
    const failing = queries.extendPause(last)
    last.keptId = 'other'
    fail(new Error('gone'))
    await failing
    expect(last.keptId).toBe('other')
  })

  it('records the rows that the backend saved on this computer', async () => {
    apiStub.executeQuery.mockImplementationOnce(
      streamed({
        results: [{ rows: [[1]], truncated: true }],
        kept: [
          { set: 0, id: 'r1:0', origin: 'spill', keptAt: 5, savedRows: 52310, savedBytes: 80 },
        ],
        elapsedMs: 900,
      }),
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(queries.stateFor('t1').panes[0]).toMatchObject({
      keptId: 'r1:0',
      kept: { origin: 'spill', keptAt: 5, savedRows: 52310, savedBytes: 80 },
      elapsedMs: 900,
    })
  })

  it('forgets the saved results that Settings cleared and keeps other kept results', async () => {
    apiStub.executeQuery.mockImplementationOnce(
      streamed({
        results: [
          { rows: [[1]], truncated: true },
          { rows: [[2]], truncated: true },
          { rows: [[3]] },
        ],
        kept: [
          { set: 0, id: 'r1:0', origin: 'spill', keptAt: 5, savedRows: 9, savedBytes: 80 },
          { set: 1, id: 'r1:1', origin: 'athena', keptAt: 5 },
        ],
      }),
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    queries.stateFor('t2')
    queries.forgetSpills()
    expect(queries.stateFor('t1').panes.map((pane) => pane.keptId)).toEqual([
      undefined,
      'r1:1',
      undefined,
    ])
  })

  it('tells that a procedure loses its later sets when the saved results reach the disk limit', async () => {
    const queries = useQueryStore()
    const ui = useUiStore()
    const notes = () => ui.notices.filter((notice) => notice.level === 'info').length
    apiStub.executeQuery.mockImplementation(streamed({ results: [{ rows: [[1]] }] }))
    useConnectionsStore().active = { c1: infoFixture('c1') }
    await queries.execute('t1', 'c1', 'EXEC report')
    expect(notes()).toBe(0)

    useSettingsStore().update({ keepFullResults: true })
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(notes()).toBe(0)
    await queries.execute('t1', 'c2', 'EXEC report')
    expect(notes()).toBe(0)
    await queries.execute('t1', 'c1', 'EXEC report')
    expect(ui.notices.slice(-1)[0]).toMatchObject({
      level: 'info',
      message:
        "If saved results reach the disk limit, the procedure's later result sets are skipped.",
    })
  })

  it('gives each cut set that the run did not save its reason', async () => {
    apiStub.executeQuery.mockImplementationOnce(
      async (_request: unknown, handlers: import('@/lib/results').ResultStreamHandlers) => {
        handlers.onSet(ResultTable.fromRows([], [[1]], true))
        handlers.onSet(ResultTable.fromRows([], [[2]], true))
        handlers.onEnd({
          messages: [],
          rowsAffected: null,
          elapsedMs: 1,
          stats: null,
          // The third set never opened a pane.
          unsaved: [
            { set: 1, reason: 'diskLimit' },
            { set: 2, reason: 'stopped' },
          ],
        })
      },
    )
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const panes = queries.stateFor('t1').panes
    expect(panes.map((pane) => pane.unsaved)).toEqual([undefined, 'diskLimit'])
  })

  it('needs no answer from a release', async () => {
    apiStub.releaseKept.mockRejectedValue(new Error('gone'))
    keptRun('r1')
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    queries.clear('t1')
    expect(await released()).toEqual(['r1:0', 'r1:1'])
  })
})
