import { beforeEach, describe, expect, it, vi } from 'vitest'
import { markRaw } from 'vue'
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

    expect(wrapper.find('[data-test="status-connection"]').text()).toBe('Server, connected')
    expect(wrapper.find('[data-test="status-dialect"]').text()).toBe('T-SQL')
  })

  it('reports that the record of a connection is gone', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    useTabsStore().add({ connectionId: 'ghost' })
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-connection"]').text()).toBe(
      'Deleted connection, not connected',
    )
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
    expect(wrapper.find('[data-test="status-health-text"]').text()).toBe(', connected')

    connections.health = { c1: ConnectionHealth.Reconnecting }
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-connection"] .mdi-lan-pending').exists()).toBe(true)
    expect(wrapper.find('[data-test="status-health-text"]').text()).toBe(', reconnecting')

    connections.health = {}
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-connection"] .mdi-lan-disconnect').exists()).toBe(true)
    expect(wrapper.find('[data-test="status-health-text"]').text()).toBe(', not connected')
  })

  it('gives no health text when no connection is chosen', () => {
    const wrapper = mountWithPlugins(StatusBar)
    expect(wrapper.find('[data-test="status-health-text"]').exists()).toBe(false)
  })

  it('reports a statement that runs', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    useQueryStore().stateFor(tab.id).running = true
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-state"]').text()).toBe('Running…')
  })

  it('reports how far the saving of all rows got', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.running = true
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-saving"]').exists()).toBe(false)
    state.saving = { rows: 1_200_000, bytes: 340 * 1024 ** 2 }
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-saving"]').text()).toBe(
      'Saving all rows: 1.2M rows, 340 MB',
    )
  })

  it('counts the time while the statement runs', async () => {
    vi.useFakeTimers()
    const start = Date.now()
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.running = true
    state.startedAt = start
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-running-elapsed"]').text()).toBe('0 ms')

    vi.advanceTimersByTime(1500)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-running-elapsed"]').text()).toBe('1.50 s')

    // The clock stops with the statement, and the final time takes its place.
    state.running = false
    state.startedAt = null
    state.elapsedMs = 1500
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-running-elapsed"]').exists()).toBe(false)

    vi.useRealTimers()
    wrapper.unmount()
  })

  it('counts no time for a run that reports no start', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.running = true
    state.startedAt = null
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-running-elapsed"]').exists()).toBe(false)
  })

  it('reports the category of a failure', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    useQueryStore().stateFor(tab.id).error = {
      category: 'database',
      message: 'no such column',
      detail: null,
    }
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-state"]').text()).toBe('Failed: database error')
    expect(wrapper.find('[data-test="status-state"]').attributes('title')).toBe('no such column')
  })

  it('names a category in words and keeps an unknown one as it came', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.error = { category: 'notConnected', message: 'gone', detail: null }
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-state"]').text()).toBe('Failed: not connected')

    state.error = { category: 'novel' as never, message: 'odd', detail: null }
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-state"]').text()).toBe('Failed: novel')
  })

  it('reports a stop that the user asked for', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.running = true
    ;(state as unknown as { stopping: boolean }).stopping = true
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-state"]').text()).toBe('Stopping…')
  })

  it('reports the rows, the time and the changes of a statement that ended', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.panes = [
      {
        id: 'p1',
        result: ResultTable.fromRows([], [[1], [2]]),
        rows: 2,
        truncated: false,
        number: 1,
        ranAt: 0,
        pinned: false,
        run: null,
      },
    ]
    state.lastRunAt = 0
    state.elapsedMs = 1500
    state.rowsAffected = 3
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-rows"]').text()).toBe('2 rows')
    expect(wrapper.find('[data-test="status-elapsed"]').text()).toBe('1.50 s')
    expect(wrapper.find('[data-test="status-affected"]').text()).toBe('3 rows affected')
  })

  it('counts the rows of the last run and not those of a kept result', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.panes = [
      {
        id: 'kept',
        result: ResultTable.fromRows([], [[1], [2], [3]]),
        rows: 3,
        truncated: false,
        number: 1,
        ranAt: 100,
        pinned: true,
        run: null,
      },
      {
        id: 'fresh',
        result: ResultTable.fromRows([], [[1], [2]]),
        rows: 2,
        truncated: false,
        number: 1,
        ranAt: 200,
        pinned: false,
        run: null,
      },
    ]
    state.lastRunAt = 200
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-rows"]').text()).toBe('2 rows')
  })

  it('counts the rows that arrive while the set streams', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.panes = [
      {
        id: 'streaming',
        // The store keeps the table raw, so Vue sees no change inside it.
        result: markRaw(ResultTable.fromRows([], [])),
        rows: 0,
        truncated: false,
        number: 1,
        ranAt: 200,
        pinned: false,
        run: null,
      },
    ]
    state.lastRunAt = 200
    await wrapper.vm.$nextTick()
    expect(wrapper.find('[data-test="status-rows"]').text()).toBe('0 rows')

    for (const pane of state.panes) {
      pane.rows = 8
    }
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-rows"]').text()).toBe('8 rows')
  })

  it('reports no rows for a run that gave no result', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.panes = [
      {
        id: 'kept',
        result: ResultTable.fromRows([], [[1]]),
        rows: 1,
        truncated: false,
        number: 1,
        ranAt: 100,
        pinned: true,
        run: null,
      },
    ]
    state.lastRunAt = 200
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[data-test="status-rows"]').exists()).toBe(false)
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

  it('says in the tooltip of the scan that a reused result cost nothing', async () => {
    const wrapper = mountWithPlugins(StatusBar)
    const tab = useTabsStore().add({ connectionId: 'c1' })
    const state = useQueryStore().stateFor(tab.id)
    state.panes = []
    state.stats = { scannedBytes: 0, engineMs: 1, queueMs: 1, resultReused: true }
    await wrapper.vm.$nextTick()

    const tooltip = wrapper
      .findAllComponents({ name: 'VTooltip' })
      .find((item) => String(item.props('text')).startsWith('Estimated cost.'))
    expect(tooltip?.props('text')).toContain('Athena reused an earlier result')
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
    expect(wrapper.find('[data-test="status-connection"]').text()).toMatch(/^Server, /)
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
