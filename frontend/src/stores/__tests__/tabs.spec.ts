import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const { parseWorkspace, useTabsStore } = await import('@/stores/tabs')
const { useConnectionsStore } = await import('@/stores/connections')

describe('parseWorkspace', () => {
  it('gives an empty workspace for a record it cannot read', () => {
    const empty = { tabs: [], activeTabId: null }
    expect(parseWorkspace(null)).toEqual(empty)
    expect(parseWorkspace('text')).toEqual(empty)
    expect(parseWorkspace({})).toEqual(empty)
    expect(parseWorkspace({ tabs: 'no' })).toEqual(empty)
  })

  it('keeps the tabs that hold an identifier and a statement', () => {
    const workspace = parseWorkspace({
      tabs: [
        {
          id: 'a',
          query: 'SELECT 1',
          title: 'One',
          connectionId: 'c1',
          dirty: true,
          savedQueryId: 'q1',
          params: [{ name: 'id', valueType: 'number', text: '7' }],
          filePath: '/data/one.sql',
        },
        { id: 'b', query: 'SELECT 2' },
        { id: 'c' },
        'nonsense',
        null,
      ],
      activeTabId: 'b',
    })
    expect(workspace.tabs).toHaveLength(2)
    expect(workspace.tabs[0]).toEqual({
      id: 'a',
      title: 'One',
      query: 'SELECT 1',
      connectionId: 'c1',
      dirty: true,
      savedQueryId: 'q1',
      params: [{ name: 'id', valueType: 'number', text: '7' }],
      filePath: '/data/one.sql',
    })
    expect(workspace.tabs[1]).toEqual({
      id: 'b',
      title: 'Query',
      query: 'SELECT 2',
      connectionId: null,
      dirty: false,
      savedQueryId: null,
      params: [],
      filePath: null,
    })
    expect(workspace.activeTabId).toBe('b')
  })

  it('falls back to the first tab when the active one is gone', () => {
    const workspace = parseWorkspace({
      tabs: [{ id: 'a', query: 'SELECT 1' }],
      activeTabId: 'missing',
    })
    expect(workspace.activeTabId).toBe('a')
  })

  it('gives no active tab when the list is empty', () => {
    expect(parseWorkspace({ tabs: [], activeTabId: 'a' }).activeTabId).toBeNull()
  })

  it('takes a file path that is text and drops one that is empty', () => {
    const workspace = parseWorkspace({
      tabs: [
        { id: 'a', query: 'SELECT 1', filePath: '' },
        { id: 'b', query: 'SELECT 2', filePath: 7 },
      ],
    })
    expect(workspace.tabs[0]?.filePath).toBeNull()
    expect(workspace.tabs[1]?.filePath).toBeNull()
  })
})

describe('tabs store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
  })

  it('opens a tab and makes it the active one', () => {
    const tabs = useTabsStore()
    const tab = tabs.add()
    expect(tabs.tabs).toHaveLength(1)
    expect(tabs.activeTabId).toBe(tab.id)
    expect(tab.title).toBe('Query 1')
    expect(tabs.hasTabs).toBe(true)
    expect(tabs.activeTab?.id).toBe(tab.id)
  })

  it('numbers each new tab in turn', () => {
    const tabs = useTabsStore()
    tabs.add()
    expect(tabs.add().title).toBe('Query 2')
  })

  it('takes the connection of the explorer when none is given', () => {
    const connections = useConnectionsStore()
    connections.select('c9')
    const tabs = useTabsStore()
    expect(tabs.add().connectionId).toBe('c9')
    expect(tabs.add({ connectionId: 'other' }).connectionId).toBe('other')
  })

  it('accepts a statement and a title', () => {
    const tabs = useTabsStore()
    const tab = tabs.add({ query: 'SELECT 1', title: 'Orders' })
    expect(tab.query).toBe('SELECT 1')
    expect(tab.title).toBe('Orders')
  })

  it('closes a tab and moves to the one before it', () => {
    const tabs = useTabsStore()
    const first = tabs.add()
    const second = tabs.add()
    tabs.close(second.id)
    expect(tabs.activeTabId).toBe(first.id)
  })

  it('leaves the active tab alone when another tab closes', () => {
    const tabs = useTabsStore()
    const first = tabs.add()
    const second = tabs.add()
    tabs.close(first.id)
    expect(tabs.activeTabId).toBe(second.id)
  })

  it('gives no active tab when the last one closes', () => {
    const tabs = useTabsStore()
    const tab = tabs.add()
    tabs.close(tab.id)
    expect(tabs.activeTabId).toBeNull()
    expect(tabs.hasTabs).toBe(false)
    expect(tabs.activeTab).toBeNull()
  })

  it('does nothing when the tab to close is not there', () => {
    const tabs = useTabsStore()
    tabs.add()
    tabs.close('missing')
    expect(tabs.tabs).toHaveLength(1)
  })

  it('raises the count of the changes for each change of a tab', () => {
    const tabs = useTabsStore()
    const start = tabs.revision
    const tab = tabs.add()
    expect(tabs.revision).toBe(start + 1)

    tabs.setQuery(tab.id, 'SELECT 2')
    tabs.rename(tab.id, 'Report')
    tabs.markClean(tab.id, 'SELECT 2')
    tabs.setParams(tab.id, [])
    tabs.setFilePath(tab.id, '/tmp/a.sql')
    tabs.activate(tab.id)
    expect(tabs.revision).toBe(start + 7)

    // A call that changes nothing raises the count no further.
    tabs.setQuery(tab.id, 'SELECT 2')
    tabs.rename(tab.id, '  ')
    tabs.activate('missing')
    tabs.close('missing')
    expect(tabs.revision).toBe(start + 7)

    tabs.close(tab.id)
    expect(tabs.revision).toBe(start + 8)
  })

  it('moves to a tab that is there and ignores one that is not', () => {
    const tabs = useTabsStore()
    const first = tabs.add()
    tabs.add()
    tabs.activate(first.id)
    expect(tabs.activeTabId).toBe(first.id)
    tabs.activate('missing')
    expect(tabs.activeTabId).toBe(first.id)
  })

  it('marks a tab as changed when its statement changes', () => {
    const tabs = useTabsStore()
    const tab = tabs.add()
    tabs.setQuery(tab.id, 'SELECT 1')
    expect(tab.query).toBe('SELECT 1')
    expect(tab.dirty).toBe(true)

    tabs.markClean(tab.id, 'SELECT 1')
    tabs.setQuery(tab.id, 'SELECT 1')
    expect(tab.dirty).toBe(false)

    tabs.setQuery('missing', 'x')
    tabs.markClean('missing', 'x')
  })

  it('clears the mark when the text comes back to the saved text', () => {
    const tabs = useTabsStore()
    const tab = tabs.add({ query: 'SELECT 1' })
    tabs.setQuery(tab.id, 'SELECT 12')
    expect(tab.dirty).toBe(true)
    tabs.setQuery(tab.id, 'SELECT 1')
    expect(tab.dirty).toBe(false)
  })

  it('keeps the mark when the text changed after the saved text went out', () => {
    const tabs = useTabsStore()
    const tab = tabs.add()
    tabs.setQuery(tab.id, 'SELECT 12')
    tabs.markClean(tab.id, 'SELECT 1')
    expect(tab.dirty).toBe(true)
    tabs.setQuery(tab.id, 'SELECT 1')
    expect(tab.dirty).toBe(false)
  })

  it('changes the connection of a tab', () => {
    const tabs = useTabsStore()
    const tab = tabs.add()
    tabs.setConnection(tab.id, 'c2')
    expect(tab.connectionId).toBe('c2')
    tabs.setConnection('missing', 'c3')
  })

  it('releases the old session when a tab moves to another connection', async () => {
    apiStub.releaseSession.mockResolvedValue(undefined)
    const tabs = useTabsStore()
    const tab = tabs.add({ connectionId: 'c1' })

    tabs.setConnection(tab.id, 'c2')
    await vi.waitFor(() => expect(apiStub.releaseSession).toHaveBeenCalledWith('c1', tab.id))

    // The same connection again, and a tab without one, release nothing.
    apiStub.releaseSession.mockClear()
    tabs.setConnection(tab.id, 'c2')
    const bare = tabs.add({ connectionId: null })
    tabs.setConnection(bare.id, 'c1')
    tabs.close(bare.id)
    expect(apiStub.releaseSession).not.toHaveBeenCalledWith('c2', tab.id)
  })

  it('releases the session of a tab that closes', async () => {
    apiStub.releaseSession.mockResolvedValue(undefined)
    const tabs = useTabsStore()
    const one = tabs.add({ connectionId: 'c1' })
    const two = tabs.add({ connectionId: 'c2' })
    const three = tabs.add({ connectionId: 'c3' })

    tabs.close(one.id)
    await vi.waitFor(() => expect(apiStub.releaseSession).toHaveBeenCalledWith('c1', one.id))

    tabs.close(three.id)
    await vi.waitFor(() => expect(apiStub.releaseSession).toHaveBeenCalledWith('c3', three.id))

    tabs.close(two.id)
    await vi.waitFor(() => expect(apiStub.releaseSession).toHaveBeenCalledWith('c2', two.id))
  })

  it('leaves the session alone when the release fails', async () => {
    apiStub.releaseSession.mockRejectedValue(new Error('gone'))
    const tabs = useTabsStore()
    const tab = tabs.add({ connectionId: 'c1' })
    tabs.close(tab.id)
    await vi.waitFor(() => expect(apiStub.releaseSession).toHaveBeenCalled())
    // The failure stays quiet; the idle reap of the backend closes it.
  })

  it('lets the results of a closed tab go', async () => {
    const { useQueryStore } = await import('@/stores/query')
    const tabs = useTabsStore()
    const queries = useQueryStore()
    const one = tabs.add()
    const two = tabs.add()
    const three = tabs.add()
    queries.stateFor(one.id)
    queries.stateFor(two.id)
    queries.stateFor(three.id)

    tabs.close(one.id)
    expect(queries.states[one.id]).toBeUndefined()
    expect(queries.states[two.id]).toBeDefined()

    tabs.close(three.id)
    expect(queries.states[three.id]).toBeUndefined()
    expect(queries.states[two.id]).toBeDefined()

    tabs.close(two.id)
    expect(queries.states[two.id]).toBeUndefined()
  })

  it('renames a tab but keeps a name that is only blank space', () => {
    const tabs = useTabsStore()
    const tab = tabs.add()
    tabs.rename(tab.id, '  Orders  ')
    expect(tab.title).toBe('Orders')
    tabs.rename(tab.id, '   ')
    expect(tab.title).toBe('Orders')
    tabs.rename('missing', 'x')
  })

  it('writes the open tabs and reads them again', async () => {
    const tabs = useTabsStore()
    const tab = tabs.add({ query: 'SELECT 1', connectionId: 'c1' })
    apiStub.saveWorkspace.mockResolvedValue(undefined)
    await tabs.persist()
    expect(apiStub.saveWorkspace).toHaveBeenCalledWith({
      tabs: [
        {
          id: tab.id,
          title: tab.title,
          query: 'SELECT 1',
          connectionId: 'c1',
          dirty: false,
          savedQueryId: null,
          params: [],
          filePath: null,
        },
      ],
      activeTabId: tab.id,
    })
  })

  it('holds the values of the parameters of one tab', () => {
    const tabs = useTabsStore()
    const tab = tabs.add({ query: 'SELECT :id' })
    const values = [{ name: 'id', valueType: 'number' as const, text: '7' }]

    tabs.setParams(tab.id, values)
    expect(tabs.tabs[0]?.params).toEqual(values)

    // A tab that is not there is left alone.
    tabs.setParams('gone', [])
    expect(tabs.tabs[0]?.params).toEqual(values)
  })

  it('keeps the tabs open when the workspace cannot be written', async () => {
    const tabs = useTabsStore()
    tabs.add()
    apiStub.saveWorkspace.mockRejectedValue(new Error('read only'))
    await tabs.persist()
    expect(tabs.tabs).toHaveLength(1)
  })

  it('continues the titles after the highest restored one', async () => {
    apiStub.getWorkspace.mockResolvedValue({
      tabs: [
        { id: 'a', query: 'SELECT 1', title: 'Query 5' },
        { id: 'b', query: 'SELECT 2', title: 'Orders' },
      ],
      activeTabId: 'a',
    })
    const tabs = useTabsStore()
    await tabs.restore()
    expect(tabs.add().title).toBe('Query 6')
  })

  it('restores the tabs of the last session', async () => {
    apiStub.getWorkspace.mockResolvedValue({
      tabs: [{ id: 'a', query: 'SELECT 1', title: 'One' }],
      activeTabId: 'a',
    })
    const tabs = useTabsStore()
    await tabs.restore()
    expect(tabs.tabs).toHaveLength(1)
    expect(tabs.tabs[0]?.dirty).toBe(false)
    expect(tabs.activeTabId).toBe('a')
    expect(tabs.add().title).toBe('Query 2')
  })

  it('starts empty when the workspace cannot be read', async () => {
    apiStub.getWorkspace.mockRejectedValue(new Error('gone'))
    const tabs = useTabsStore()

    await tabs.restore()

    expect(tabs.tabs).toEqual([])
    expect(tabs.activeTabId).toBeNull()
  })

  it('restores the file that a tab came from', async () => {
    apiStub.getWorkspace.mockResolvedValue({
      tabs: [{ id: 'a', query: 'SELECT 1', filePath: '/data/a.sql' }],
      activeTabId: 'a',
    })
    apiStub.readTextFile.mockResolvedValue('SELECT 1')
    const tabs = useTabsStore()

    await tabs.restore()

    expect(tabs.tabs[0]?.filePath).toBe('/data/a.sql')
    expect(tabs.tabs[0]?.dirty).toBe(false)
  })

  it('restores the mark of a tab that the last session did not save', async () => {
    apiStub.getWorkspace.mockResolvedValue({
      tabs: [{ id: 'a', query: 'SELECT 1', dirty: true }],
      activeTabId: 'a',
    })
    const tabs = useTabsStore()

    await tabs.restore()

    expect(tabs.tabs[0]?.dirty).toBe(true)
    expect(apiStub.readTextFile).not.toHaveBeenCalled()

    // The text that the last session saved is unknown, so the mark stays.
    tabs.setQuery('a', 'SELECT 2')
    tabs.setQuery('a', 'SELECT 1')
    expect(tabs.tabs[0]?.dirty).toBe(true)
  })

  it('clears the mark of a restored tab whose text comes back', async () => {
    apiStub.getWorkspace.mockResolvedValue({
      tabs: [
        { id: 'a', query: 'SELECT 1' },
        { id: 'b', query: 'SELECT 3', filePath: '/data/b.sql' },
      ],
      activeTabId: 'a',
    })
    apiStub.readTextFile.mockResolvedValue('SELECT 2')
    const tabs = useTabsStore()

    await tabs.restore()
    expect(tabs.tabs[1]?.dirty).toBe(true)

    tabs.setQuery('a', 'SELECT 9')
    tabs.setQuery('a', 'SELECT 1')
    tabs.setQuery('b', 'SELECT 2')
    expect(tabs.tabs[0]?.dirty).toBe(false)
    expect(tabs.tabs[1]?.dirty).toBe(false)
  })

  it('marks a restored tab whose text differs from the file', async () => {
    apiStub.getWorkspace.mockResolvedValue({
      tabs: [{ id: 'a', query: 'SELECT 2', filePath: '/data/a.sql' }],
      activeTabId: 'a',
    })
    apiStub.readTextFile.mockResolvedValue('SELECT 1')
    const tabs = useTabsStore()

    await tabs.restore()

    expect(tabs.tabs[0]?.dirty).toBe(true)
  })

  it('keeps the recorded mark when the file cannot be read', async () => {
    apiStub.getWorkspace.mockResolvedValue({
      tabs: [
        { id: 'a', query: 'SELECT 1', filePath: '/data/a.sql', dirty: true },
        { id: 'b', query: 'SELECT 2', filePath: '/data/b.sql' },
      ],
      activeTabId: 'a',
    })
    apiStub.readTextFile.mockRejectedValue(new Error('outside the folders'))
    const tabs = useTabsStore()

    await tabs.restore()

    expect(tabs.tabs[0]?.dirty).toBe(true)
    expect(tabs.tabs[1]?.dirty).toBe(false)
  })

  it('holds the file of a tab in the record it writes', async () => {
    apiStub.saveWorkspace.mockResolvedValue(undefined)
    const tabs = useTabsStore()
    const tab = tabs.add({ filePath: '/data/a.sql' })

    expect(tabs.snapshot()).toEqual(
      expect.objectContaining({
        tabs: [expect.objectContaining({ id: tab.id, filePath: '/data/a.sql' })],
      }),
    )
  })

  it('sets and clears the file that a tab writes back to', () => {
    const tabs = useTabsStore()
    const tab = tabs.add()
    expect(tab.filePath).toBeNull()

    tabs.setFilePath(tab.id, '/data/a.sql')
    expect(tabs.tabs[0]?.filePath).toBe('/data/a.sql')
    expect(tabs.tabForFile('/data/a.sql')?.id).toBe(tab.id)

    tabs.setFilePath(tab.id, null)
    expect(tabs.tabs[0]?.filePath).toBeNull()
    expect(tabs.tabForFile('/data/a.sql')).toBeUndefined()

    // A tab that is gone changes nothing.
    tabs.setFilePath('nowhere', '/data/b.sql')
    expect(tabs.tabForFile('/data/b.sql')).toBeUndefined()
  })
})
