import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const { useHistoryStore, HISTORY_LIMIT, HISTORY_TEXT_BUDGET, trimHistory } =
  await import('@/stores/history')
const { useUiStore } = await import('@/stores/ui')

function entry(id: string, query: string, connectionName = 'Server') {
  return {
    id,
    connectionId: 'c1',
    connectionName,
    query,
    ranAt: '2026-08-10T00:00:00Z',
    elapsedMs: 5,
    rowCount: 1,
    succeeded: true,
    error: null,
  }
}

describe('history store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getHistory.mockResolvedValue([])
  })

  it('reads the history', async () => {
    apiStub.getHistory.mockResolvedValue([entry('h1', 'SELECT 1')])
    const history = useHistoryStore()
    await history.load()
    expect(history.entries).toHaveLength(1)
    expect(history.loading).toBe(false)
  })

  it('reports a failure to read', async () => {
    apiStub.getHistory.mockRejectedValue({ category: 'storage', message: 'no', detail: null })
    const history = useHistoryStore()
    await history.load()
    expect(useUiStore().notices[0]?.level).toBe('error')
  })

  it('filters the history by statement and by connection', async () => {
    apiStub.getHistory.mockResolvedValue([
      entry('h1', 'SELECT one'),
      entry('h2', 'SELECT two', 'Reporting'),
    ])
    const history = useHistoryStore()
    await history.load()
    expect(history.visibleEntries).toHaveLength(2)

    history.filter = 'two'
    expect(history.visibleEntries.map((item) => item.id)).toEqual(['h2'])

    history.filter = 'reporting'
    expect(history.visibleEntries.map((item) => item.id)).toEqual(['h2'])

    history.filter = '   '
    expect(history.visibleEntries).toHaveLength(2)
  })

  it('writes one execution to the history', async () => {
    apiStub.addHistoryEntry.mockResolvedValue(undefined)
    const history = useHistoryStore()
    await history.record({
      connectionId: 'c1',
      connectionName: 'Server',
      query: 'SELECT 1',
      elapsedMs: 5,
      rowCount: 1,
      succeeded: true,
      error: null,
    })
    expect(apiStub.addHistoryEntry).toHaveBeenCalledWith(
      expect.objectContaining({ query: 'SELECT 1', succeeded: true }),
    )
    expect(history.entries).toHaveLength(1)
    expect(history.entries[0]?.query).toBe('SELECT 1')
  })

  it('replaces the entry at the front when the statement repeats', async () => {
    apiStub.addHistoryEntry.mockResolvedValue(undefined)
    apiStub.getHistory.mockResolvedValue([entry('h1', 'SELECT 1'), entry('h0', 'SELECT 0')])
    const history = useHistoryStore()
    await history.load()
    await history.record({
      connectionId: 'c1',
      connectionName: 'Server',
      query: 'SELECT 1',
      elapsedMs: 9,
      rowCount: 2,
      succeeded: true,
      error: null,
    })
    expect(history.entries.map((item) => item.query)).toEqual(['SELECT 1', 'SELECT 0'])
    expect(history.entries[0]?.elapsedMs).toBe(9)
  })

  it('keeps a new entry when the statement at the front is from another connection', async () => {
    apiStub.addHistoryEntry.mockResolvedValue(undefined)
    apiStub.getHistory.mockResolvedValue([{ ...entry('h1', 'SELECT 1'), connectionId: 'c2' }])
    const history = useHistoryStore()
    await history.load()
    await history.record({
      connectionId: 'c1',
      connectionName: 'Server',
      query: 'SELECT 1',
      elapsedMs: 5,
      rowCount: 1,
      succeeded: true,
      error: null,
    })
    expect(history.entries).toHaveLength(2)
  })

  it('drops the entries above the limit', async () => {
    apiStub.addHistoryEntry.mockResolvedValue(undefined)
    apiStub.getHistory.mockResolvedValue(
      Array.from({ length: HISTORY_LIMIT }, (_unused, index) =>
        entry(`h${index}`, `SELECT ${index}`),
      ),
    )
    const history = useHistoryStore()
    await history.load()
    await history.record({
      connectionId: 'c1',
      connectionName: 'Server',
      query: 'SELECT new',
      elapsedMs: 5,
      rowCount: 1,
      succeeded: true,
      error: null,
    })
    expect(history.entries).toHaveLength(HISTORY_LIMIT)
    expect(history.entries[0]?.query).toBe('SELECT new')
    expect(history.entries[HISTORY_LIMIT - 1]?.query).toBe(`SELECT ${HISTORY_LIMIT - 2}`)
  })

  it('drops the older entries whose text passes the budget', async () => {
    apiStub.addHistoryEntry.mockResolvedValue(undefined)
    const quarter = 'x'.repeat(HISTORY_TEXT_BUDGET / 4)
    apiStub.getHistory.mockResolvedValue(
      ['a', 'b', 'c'].map((name) => entry(`h${name}`, `${name}${quarter}`)),
    )
    const history = useHistoryStore()
    await history.load()
    await history.record({
      connectionId: 'c1',
      connectionName: 'Server',
      query: 'SELECT 1',
      elapsedMs: 5,
      rowCount: 0,
      succeeded: false,
      error: quarter,
    })
    // The new entry and two of the old ones fit, because the error text
    // counts against the budget too.
    expect(history.entries).toHaveLength(3)
    expect(history.entries.slice(1).map((item) => item.id)).toEqual(['ha', 'hb'])
  })

  it('keeps the newest entry when its text alone passes the budget', () => {
    const huge = entry('big', 'x'.repeat(HISTORY_TEXT_BUDGET + 1))
    expect(trimHistory([huge, entry('h1', 'SELECT 1')])).toEqual([huge])
    expect(trimHistory([])).toEqual([])
  })

  it('keeps an entry for this session when it cannot be written', async () => {
    apiStub.addHistoryEntry.mockRejectedValue(new Error('read only'))
    const history = useHistoryStore()
    await history.record({
      connectionId: 'c1',
      connectionName: 'Server',
      query: 'SELECT 1',
      elapsedMs: 5,
      rowCount: 0,
      succeeded: false,
      error: 'no such table',
    })
    expect(history.entries).toHaveLength(1)
    expect(history.entries[0]?.error).toBe('no such table')
    expect(useUiStore().notices).toHaveLength(0)
  })

  it('empties the history', async () => {
    apiStub.getHistory.mockResolvedValue([entry('h1', 'SELECT 1')])
    apiStub.clearHistory.mockResolvedValue(undefined)
    const history = useHistoryStore()
    await history.load()
    await history.clear()
    expect(history.entries).toEqual([])
    expect(useUiStore().notices[0]?.level).toBe('success')
  })

  it('reports a failure to empty the history', async () => {
    apiStub.clearHistory.mockRejectedValue({ category: 'storage', message: 'no', detail: null })
    const history = useHistoryStore()
    await history.clear()
    expect(useUiStore().notices[0]?.level).toBe('error')
  })
})
