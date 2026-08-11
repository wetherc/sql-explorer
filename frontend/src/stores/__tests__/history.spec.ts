import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const { useHistoryStore, HISTORY_LIMIT } = await import('@/stores/history')
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

function saved(id: string, name: string, query = 'SELECT 1', folder: string | null = null) {
  return { id, name, query, connectionId: null, folder, updatedAt: '2026-08-10T00:00:00Z' }
}

describe('history store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getHistory.mockResolvedValue([])
    apiStub.getSavedQueries.mockResolvedValue([])
  })

  it('reads the history and the saved statements', async () => {
    apiStub.getHistory.mockResolvedValue([entry('h1', 'SELECT 1')])
    apiStub.getSavedQueries.mockResolvedValue([saved('q1', 'Daily')])
    const history = useHistoryStore()
    await history.load()
    expect(history.entries).toHaveLength(1)
    expect(history.savedQueries).toHaveLength(1)
    expect(history.loading).toBe(false)
  })

  it('reports a failure to read', async () => {
    apiStub.getHistory.mockRejectedValue({ kind: 'storage', message: 'no', detail: null })
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

  it('filters the saved statements by name and by text', async () => {
    apiStub.getSavedQueries.mockResolvedValue([
      saved('q1', 'Daily count', 'SELECT COUNT(*)'),
      saved('q2', 'Orders', 'SELECT * FROM orders'),
    ])
    const history = useHistoryStore()
    await history.load()
    expect(history.visibleSavedQueries).toHaveLength(2)

    history.filter = 'daily'
    expect(history.visibleSavedQueries.map((item) => item.id)).toEqual(['q1'])

    history.filter = 'orders'
    expect(history.visibleSavedQueries.map((item) => item.id)).toEqual(['q2'])
  })

  it('groups the saved statements by their folder', async () => {
    apiStub.getSavedQueries.mockResolvedValue([
      saved('q1', 'A', 'SELECT 1', 'Reports'),
      saved('q2', 'B'),
      saved('q3', 'C', 'SELECT 1', '  '),
    ])
    const history = useHistoryStore()
    await history.load()
    expect(history.folders).toEqual(['Reports', 'Saved queries'])
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
    apiStub.clearHistory.mockRejectedValue({ kind: 'storage', message: 'no', detail: null })
    const history = useHistoryStore()
    await history.clear()
    expect(useUiStore().notices[0]?.level).toBe('error')
  })

  it('refuses to save a statement without a name', async () => {
    const history = useHistoryStore()
    expect(await history.save({ name: '  ', query: 'SELECT 1' })).toBeNull()
    expect(apiStub.saveQuery).not.toHaveBeenCalled()
    expect(useUiStore().notices[0]?.level).toBe('warning')
  })

  it('saves a statement under a name and a folder', async () => {
    apiStub.saveQuery.mockResolvedValue(undefined)
    apiStub.getSavedQueries.mockResolvedValue([saved('q1', 'Daily')])
    const history = useHistoryStore()
    const record = await history.save({
      name: ' Daily ',
      query: 'SELECT 1',
      connectionId: 'c1',
      folder: ' Reports ',
    })
    expect(record?.name).toBe('Daily')
    expect(record?.folder).toBe('Reports')
    expect(history.savedQueries).toHaveLength(1)
    expect(useUiStore().notices[0]?.level).toBe('success')
  })

  it('keeps the identifier of a statement it saves again', async () => {
    apiStub.saveQuery.mockResolvedValue(undefined)
    const history = useHistoryStore()
    const record = await history.save({ id: 'q1', name: 'Daily', query: 'SELECT 2' })
    expect(record?.id).toBe('q1')
  })

  it('leaves the folder empty when none is given', async () => {
    apiStub.saveQuery.mockResolvedValue(undefined)
    const history = useHistoryStore()
    const record = await history.save({ name: 'Daily', query: 'SELECT 1' })
    expect(record?.folder).toBeNull()
    expect(record?.connectionId).toBeNull()
  })

  it('reports a failure to save a statement', async () => {
    apiStub.saveQuery.mockRejectedValue({ kind: 'storage', message: 'no', detail: null })
    const history = useHistoryStore()
    expect(await history.save({ name: 'Daily', query: 'SELECT 1' })).toBeNull()
  })

  it('removes a saved statement', async () => {
    apiStub.getSavedQueries.mockResolvedValue([saved('q1', 'A'), saved('q2', 'B')])
    apiStub.deleteSavedQuery.mockResolvedValue(undefined)
    const history = useHistoryStore()
    await history.load()
    await history.remove('q1')
    expect(history.savedQueries.map((item) => item.id)).toEqual(['q2'])
  })

  it('reports a failure to remove a saved statement', async () => {
    apiStub.deleteSavedQuery.mockRejectedValue({ kind: 'storage', message: 'no', detail: null })
    const history = useHistoryStore()
    await history.remove('q1')
    expect(useUiStore().notices[0]?.level).toBe('error')
  })
})
