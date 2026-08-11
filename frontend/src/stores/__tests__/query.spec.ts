import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub, connectionFixture } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const { newQueryState, totalRows, useQueryStore } = await import('@/stores/query')
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
    })
  })
})

describe('totalRows', () => {
  it('counts the rows of every result set', () => {
    expect(totalRows([])).toBe(0)
    expect(
      totalRows([
        { columns: [], rows: [[1], [2]], truncated: false },
        { columns: [], rows: [[3]], truncated: false },
      ]),
    ).toBe(3)
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

  it('refuses an empty statement', async () => {
    const queries = useQueryStore()
    expect(await queries.execute('t1', 'c1', '   ')).toBe(false)
    expect(apiStub.executeQuery).not.toHaveBeenCalled()
    expect(useUiStore().notices[0]?.message).toBe('There is no statement to run.')
  })

  it('sends the statement to the backend and keeps the result', async () => {
    apiStub.executeQuery.mockResolvedValue(response())
    const settings = useSettingsStore()
    settings.update({ maxRows: 25 })
    const connections = useConnectionsStore()
    await connections.load()

    const queries = useQueryStore()
    expect(await queries.execute('t1', 'c1', ' SELECT 1 ')).toBe(true)

    expect(apiStub.executeQuery).toHaveBeenCalledWith({
      connectionId: 'c1',
      requestId: expect.any(String),
      query: 'SELECT 1',
      tabId: 't1',
      queryParams: undefined,
      options: { maxRows: 25, timeoutSecs: 300 },
    })

    const state = queries.stateFor('t1')
    expect(state.panes).toHaveLength(1)
    expect(state.panes[0]?.number).toBe(1)
    expect(state.activePaneId).toBe(state.panes[0]?.id)
    expect(state.messages).toEqual([{ level: 'info', text: '1 row returned.', detail: null }])
    expect(state.elapsedMs).toBe(12)
    expect(state.running).toBe(false)
    expect(state.requestId).toBeNull()
  })

  it('uses the default time limit for a connection it does not know', async () => {
    apiStub.executeQuery.mockResolvedValue(response())
    const queries = useQueryStore()
    await queries.execute('t1', 'unknown', 'SELECT 1')
    expect(apiStub.executeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ options: { maxRows: 10000, timeoutSecs: 300 } }),
    )
  })

  it('warns when the row limit stopped the read', async () => {
    apiStub.executeQuery.mockResolvedValue({
      ...response(),
      results: [{ columns: [], rows: [[1]], truncated: true }],
    })
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
    release(response())
    await first
  })

  it('keeps the reason a statement failed', async () => {
    apiStub.executeQuery.mockRejectedValue({
      kind: 'database',
      message: 'no such column',
      detail: null,
    })
    const queries = useQueryStore()
    expect(await queries.execute('t1', 'c1', 'SELECT bad')).toBe(false)
    expect(queries.stateFor('t1').error?.message).toBe('no such column')
    expect(queries.stateFor('t1').running).toBe(false)
  })

  it('reports a length of time even when the start is no longer known', async () => {
    apiStub.executeQuery.mockImplementation(async () => {
      useQueryStore().stateFor('t1').startedAt = null
      throw { kind: 'database', message: 'no', detail: null }
    })
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(queries.stateFor('t1').elapsedMs).toBe(0)
  })

  it('writes every execution to the history', async () => {
    apiStub.executeQuery.mockResolvedValue(response())
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
    apiStub.executeQuery.mockResolvedValue(response())
    const history = useHistoryStore()
    const record = vi.spyOn(history, 'record')
    const queries = useQueryStore()
    await queries.execute('t1', 'lost', 'SELECT 1')
    expect(record).toHaveBeenCalledWith(
      expect.objectContaining({ connectionName: 'Connection that is gone' }),
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
      kind: 'estimated',
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
    expect(apiStub.explainQuery).toHaveBeenCalledWith(expect.objectContaining({ kind: 'actual' }))
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
    release(response())
    await running
  })

  it('remembers the statement and the values of the last run', async () => {
    apiStub.executeQuery.mockResolvedValue(response())
    apiStub.explainQuery.mockResolvedValue(response())
    const queries = useQueryStore()

    await queries.execute('t1', 'c1', ' SELECT :id ', { id: 7 })
    expect(queries.stateFor('t1').lastRun).toEqual({ query: 'SELECT :id', params: { id: 7 } })

    // A plan is not a run, so it does not replace the last run.
    await queries.explain('t1', 'c1', 'SELECT 2', 'estimated')
    expect(queries.stateFor('t1').lastRun?.query).toBe('SELECT :id')

    // A run that failed leaves the last good run in place.
    apiStub.executeQuery.mockRejectedValue({ kind: 'database', message: 'no', detail: null })
    await queries.execute('t1', 'c1', 'SELECT bad')
    expect(queries.stateFor('t1').lastRun?.query).toBe('SELECT :id')
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
    release(response())
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
    expect(useUiStore().notices.some((notice) => notice.level === 'warning')).toBe(true)
    release(response())
    await running
  })

  it('moves to a result that is there, and to the messages', async () => {
    apiStub.executeQuery.mockResolvedValue(twoResults())
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
    apiStub.executeQuery.mockResolvedValue(twoResults())
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
    apiStub.executeQuery.mockResolvedValue(twoResults())
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    queries.togglePin('t1', state.panes[0]!.id)
    await queries.execute('t1', 'c1', 'SELECT 2')
    expect(state.panes.map((pane) => pane.number)).toEqual([1, 1, 2])
  })

  it('refuses to keep more results than the settings allow', async () => {
    apiStub.executeQuery.mockResolvedValue(twoResults())
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
    apiStub.executeQuery.mockResolvedValue(twoResults())
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    queries.togglePin('t1', 'no-such-result')
    expect(queries.stateFor('t1').panes.every((pane) => !pane.pinned)).toBe(true)
  })

  it('closes a result and shows the one that takes its place', async () => {
    apiStub.executeQuery.mockResolvedValue(twoResults())
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
    apiStub.executeQuery.mockResolvedValue(twoResults())
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    const last = state.panes[1]!.id
    queries.closePane('t1', last)
    expect(state.activePaneId).toBe(state.panes[0]?.id)
  })

  it('keeps the result on show when another one closes', async () => {
    apiStub.executeQuery.mockResolvedValue(twoResults())
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    const state = queries.stateFor('t1')
    const second = state.panes[1]!.id

    queries.selectPane('t1', second)
    queries.closePane('t1', state.panes[0]!.id)
    expect(state.activePaneId).toBe(second)
  })

  it('records the scan of a statement and adds it to the session', async () => {
    apiStub.executeQuery.mockResolvedValue({
      ...response(),
      stats: { scannedBytes: 1024 ** 3, engineMs: 120, queueMs: 3, resultReused: false },
    })
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    await queries.execute('t2', 'c1', 'SELECT 2')

    expect(queries.stateFor('t1').stats?.scannedBytes).toBe(1024 ** 3)
    expect(queries.sessionScannedBytes).toBe(2 * 1024 ** 3)
  })

  it('warns about a scan above the limit of the settings', async () => {
    useSettingsStore().update({ athenaScanWarningGb: 1, athenaPricePerTerabyte: 5 })
    apiStub.executeQuery.mockResolvedValue({
      ...response(),
      stats: { scannedBytes: 2 * 1024 ** 3, engineMs: null, queueMs: null, resultReused: null },
    })
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')

    const notice = useUiStore().notices.find((item) => item.level === 'warning')
    expect(notice?.message).toContain('warning limit of 1 GB')
    expect(notice?.detail).toContain('$0.01')
  })

  it('counts nothing for an engine that reports no scan', async () => {
    apiStub.executeQuery.mockResolvedValue(response())
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    expect(queries.stateFor('t1').stats).toBeNull()
    expect(queries.sessionScannedBytes).toBe(0)
  })

  it('closes nothing for a result that is not there', async () => {
    apiStub.executeQuery.mockResolvedValue(twoResults())
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
