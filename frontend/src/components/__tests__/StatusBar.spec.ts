import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture, infoFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const StatusBar = (await import('@/components/StatusBar.vue')).default
const { mountWithPlugins } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useQueryStore } = await import('@/stores/query')
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

  it('names a connection the store does not know by its identifier', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    useTabsStore().add({ connectionId: 'ghost' })
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-connection"]').text()).toBe('ghost')
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
    state.results = [{ columns: [], rows: [[1], [2]], truncated: false }]
    state.elapsedMs = 1500
    state.rowsAffected = 3
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-rows"]').text()).toBe('2 rows')
    expect(wrapper.find('[data-test="status-elapsed"]').text()).toBe('1.50 s')
    expect(wrapper.find('[data-test="status-affected"]').text()).toBe('3 rows affected')
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
