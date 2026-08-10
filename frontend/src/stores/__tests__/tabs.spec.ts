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
          savedQueryId: 'q1',
          params: [{ name: 'id', kind: 'number', text: '7' }],
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
      savedQueryId: 'q1',
      params: [{ name: 'id', kind: 'number', text: '7' }],
    })
    expect(workspace.tabs[1]).toEqual({
      id: 'b',
      title: 'Query',
      query: 'SELECT 2',
      connectionId: null,
      savedQueryId: null,
      params: [],
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

  it('closes every other tab', () => {
    const tabs = useTabsStore()
    tabs.add()
    const kept = tabs.add()
    tabs.add()
    tabs.closeOthers(kept.id)
    expect(tabs.tabs).toHaveLength(1)
    expect(tabs.activeTabId).toBe(kept.id)
  })

  it('closes every tab', () => {
    const tabs = useTabsStore()
    tabs.add()
    tabs.closeAll()
    expect(tabs.tabs).toEqual([])
    expect(tabs.activeTabId).toBeNull()
    tabs.closeOthers('missing')
    expect(tabs.activeTabId).toBeNull()
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

    tabs.markClean(tab.id)
    tabs.setQuery(tab.id, 'SELECT 1')
    expect(tab.dirty).toBe(false)

    tabs.setQuery('missing', 'x')
    tabs.markClean('missing')
  })

  it('changes the connection of a tab', () => {
    const tabs = useTabsStore()
    const tab = tabs.add()
    tabs.setConnection(tab.id, 'c2')
    expect(tab.connectionId).toBe('c2')
    tabs.setConnection('missing', 'c3')
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

    tabs.closeOthers(two.id)
    expect(queries.states[three.id]).toBeUndefined()
    expect(queries.states[two.id]).toBeDefined()

    tabs.closeAll()
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
          savedQueryId: null,
          params: [],
        },
      ],
      activeTabId: tab.id,
    })
  })

  it('holds the values of the parameters of one tab', () => {
    const tabs = useTabsStore()
    const tab = tabs.add({ query: 'SELECT :id' })
    const values = [{ name: 'id', kind: 'number' as const, text: '7' }]

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
})
