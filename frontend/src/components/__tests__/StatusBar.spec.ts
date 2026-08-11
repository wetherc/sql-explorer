import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture, infoFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const StatusBar = (await import('@/components/StatusBar.vue')).default
const { mountWithPlugins } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useQueryStore } = await import('@/stores/query')
const { ResultTable } = await import('@/lib/results')
const { useTabsStore } = await import('@/stores/tabs')
const { ConnectionHealth, Dialect } = await import('@/types/api')

describe('StatusBar', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
  })

  it('reports that there is no connection', () => {
    const wrapper = mountWithPlugins(StatusBar)
    expect(wrapper.find('[data-test="status-connection"]').text()).toBe('No connection')
    expect(wrapper.find('[data-test="status-state"]').text()).toBe('Ready')
    expect(wrapper.find('[data-test="status-dialect"]').exists()).toBe(false)
  })

  it('names the connection of the active tab and its dialect', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const connections = useConnectionsStore()
    await connections.load()
    const tabs = useTabsStore()
    tabs.add({ connectionId: 'c1' })
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-connection"]').text()).toBe('Server')
    expect(wrapper.find('[data-test="status-dialect"]').text()).toBe('T-SQL')
  })

  it('reports that the record of a connection is gone', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    useTabsStore().add({ connectionId: 'ghost' })
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-connection"]').text()).toBe('Connection that is gone')
    expect(wrapper.find('[data-test="status-dialect"]').text()).toBe('SQL')
  })

  it('names each dialect in the terms the engine uses', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const connections = useConnectionsStore()
    await connections.load()
    const tabs = useTabsStore()
    tabs.add({ connectionId: 'c1' })

    for (const [dialect, label] of [
      [Dialect.MySql, 'MySQL'],
      [Dialect.Postgres, 'PostgreSQL'],
      [Dialect.Sqlite, 'SQLite'],
      [Dialect.Athena, 'Athena'],
    ] as const) {
      connections.active = { c1: { ...infoFixture(), dialect } }
      await wrapper.vm.$nextTick()
      expect(wrapper.find('[data-test="status-dialect"]').text()).toBe(label)
    }
  })

  it('reports the state of the connection', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const connections = useConnectionsStore()
    await connections.load()
    useTabsStore().add({ connectionId: 'c1' })
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-connection"] .mdi-lan-connect').exists()).toBe(true)

    connections.health = { c1: ConnectionHealth.Reconnecting }
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-connection"] .mdi-lan-disconnect').exists()).toBe(true)
  })

  it('reports a statement that runs', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    useQueryStore().stateFor(tab.id).running = true
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-state"]').text()).toBe('Running…')
  })

  it('reports the kind of a failure', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    useQueryStore().stateFor(tab.id).error = {
      kind: 'database',
      message: 'no such column',
      detail: null,
    }
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-state"]').text()).toBe('Failed: database')
  })

  it('reports the rows, the time and the changes of a statement that ended', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.panes = [
      {
        id: 'p1',
        result: ResultTable.fromRows([], [[1], [2]]),
        number: 1,
        ranAt: 0,
        pinned: false,
      },
    ]
    state.elapsedMs = 1500
    state.rowsAffected = 3
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-rows"]').text()).toBe('2 rows')
    expect(wrapper.find('[data-test="status-elapsed"]').text()).toBe('1.50 s')
    expect(wrapper.find('[data-test="status-affected"]').text()).toBe('3 rows affected')
  })

  it('reports the scan, its estimated cost and the total of the session', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const queries = useQueryStore()
    const state = queries.stateFor(tab.id)
    state.panes = []
    state.stats = {
      scannedBytes: 1024 ** 4 / 2,
      engineMs: 100,
      queueMs: 1,
      resultReused: false,
    }
    queries.sessionScannedBytes = 1024 ** 4
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-scan"]').text()).toBe('512.00 GB scanned, $2.50 est.')
    expect(wrapper.find('[data-test="status-session-cost"]').text()).toBe('$5.00 this session')
  })

  it('says nothing about a scan for an engine that reports none', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    useTabsStore().add({ connectionId: 'c1' })
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-scan"]').exists()).toBe(false)
  })

  it('falls back to the connection of the explorer when no tab is open', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-connection"]').text()).toBe('Server')
  })
})

describe('StatusBar without a tab', () => {
  it('reports no rows when no tab is open', () => {
    const wrapper = mountWithPlugins(StatusBar)
    expect(wrapper.find('[data-test="status-rows"]').exists()).toBe(false)
    expect(wrapper.find('[data-test="status-affected"]').exists()).toBe(false)
  })
})

describe('StatusBar as a part a reader can follow', () => {
  it('tells a reader of each change of the state of a run', () => {
    const wrapper = mountWithPlugins(StatusBar)
    const state = wrapper.find('[data-test="status-state"]')

    expect(state.attributes('role')).toBe('status')
    expect(state.attributes('aria-live')).toBe('polite')
  })
})
