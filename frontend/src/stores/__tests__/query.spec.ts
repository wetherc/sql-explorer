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
    messages: ['1 row returned.'],
    rowsAffected: null,
    elapsedMs: 12,
  }
}

describe('newQueryState', () => {
  it('starts at rest', () => {
    expect(newQueryState()).toEqual({
      running: false,
      requestId: null,
      error: null,
      results: [],
      messages: [],
      rowsAffected: null,
      elapsedMs: 0,
      startedAt: null,
      activeResultIndex: 0,
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
      options: { maxRows: 25, timeoutSecs: 300 },
    })

    const state = queries.stateFor('t1')
    expect(state.results).toHaveLength(1)
    expect(state.messages).toEqual(['1 row returned.'])
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

  it('names the connection by its identifier when the record is gone', async () => {
    apiStub.executeQuery.mockResolvedValue(response())
    const history = useHistoryStore()
    const record = vi.spyOn(history, 'record')
    const queries = useQueryStore()
    await queries.execute('t1', 'lost', 'SELECT 1')
    expect(record).toHaveBeenCalledWith(expect.objectContaining({ connectionName: 'lost' }))
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

    await queries.cancel('t1', 'c1')
    expect(apiStub.cancelQuery).toHaveBeenCalledWith('c1', requestId)
    release(response())
    await running
  })

  it('does nothing when there is no statement to stop', async () => {
    const queries = useQueryStore()
    await queries.cancel('t1', 'c1')
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
    await queries.cancel('t1', 'c1')
    expect(useUiStore().notices.some((notice) => notice.level === 'warning')).toBe(true)
    release(response())
    await running
  })

  it('moves to a result set that is there', async () => {
    apiStub.executeQuery.mockResolvedValue({
      ...response(),
      results: [
        { columns: [], rows: [], truncated: false },
        { columns: [], rows: [], truncated: false },
      ],
    })
    const queries = useQueryStore()
    await queries.execute('t1', 'c1', 'SELECT 1')
    queries.selectResult('t1', 1)
    expect(queries.stateFor('t1').activeResultIndex).toBe(1)
    queries.selectResult('t1', 9)
    expect(queries.stateFor('t1').activeResultIndex).toBe(1)
    queries.selectResult('t1', -1)
    expect(queries.stateFor('t1').activeResultIndex).toBe(1)
  })
})
