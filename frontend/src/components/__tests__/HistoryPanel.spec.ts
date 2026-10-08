import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture, infoFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const HistoryPanel = (await import('@/components/HistoryPanel.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useHistoryStore } = await import('@/stores/history')
const { useTabsStore } = await import('@/stores/tabs')

const entry = {
  id: 'h1',
  connectionId: 'c1',
  connectionName: 'Server',
  query: 'SELECT 1',
  ranAt: '2026-08-10T00:00:00Z',
  elapsedMs: 5,
  rowCount: 1,
  succeeded: true,
  error: null,
}

describe('HistoryPanel', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.getHistory.mockResolvedValue([])
  })

  it('says so when nothing has run', () => {
    const wrapper = mountWithPlugins(HistoryPanel)
    expect(wrapper.text()).toContain('No history yet')
  })

  it('lists the statements that ran, with the facts of each one', async () => {
    apiStub.getHistory.mockResolvedValue([entry])
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()

    const row = wrapper.find('[data-test="history-entry"]')
    expect(row.text()).toContain('SELECT 1')
    expect(row.text()).toContain('Server')
    expect(row.text()).toContain('5 ms')
    expect(row.text()).toContain('1 row')
  })

  it('marks a statement that failed', async () => {
    apiStub.getHistory.mockResolvedValue([{ ...entry, succeeded: false }])
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.mdi-alert-circle-outline').exists()).toBe(true)
  })

  it('opens a past statement in a tab on its own connection', async () => {
    apiStub.getHistory.mockResolvedValue([entry])
    const wrapper = mountWithPlugins(HistoryPanel)
    await useConnectionsStore().load()
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="history-entry"]').trigger('click')
    const tabs = useTabsStore()
    expect(tabs.tabs[0]?.query).toBe('SELECT 1')
    expect(tabs.tabs[0]?.connectionId).toBe('c1')
  })

  it('opens a past statement on the selected connection when its own is closed', async () => {
    apiStub.getHistory.mockResolvedValue([{ ...entry, connectionId: 'gone' }])
    const wrapper = mountWithPlugins(HistoryPanel)
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="history-entry"]').trigger('click')
    expect(useTabsStore().tabs[0]?.connectionId).toBe('c1')
  })

  it('empties the history on request', async () => {
    apiStub.getHistory.mockResolvedValue([entry])
    apiStub.clearHistory.mockResolvedValue(undefined)
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="clear-history"]').trigger('click')
    await settle()
    expect(document.body.textContent).toContain('This removes every statement from the history.')
    // The history is emptied only once the user answers the question.
    expect(apiStub.clearHistory).not.toHaveBeenCalled()

    const confirm = document.querySelector('[data-test="confirm-accept"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(apiStub.clearHistory).toHaveBeenCalled()
  })

  it('keeps only the entries that match the filter', async () => {
    apiStub.getHistory.mockResolvedValue([entry, { ...entry, id: 'h2', query: 'SELECT 2' }])
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="history-filter"] input').setValue('SELECT 2')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('[data-test="history-entry"]')).toHaveLength(1)
  })
})

describe('HistoryPanel asking before it takes something away', () => {
  it('keeps the history when the question is refused', async () => {
    apiStub.getHistory.mockResolvedValue([entry])
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="clear-history"]').trigger('click')
    await settle()
    const cancel = document.querySelector('[data-test="confirm-cancel"]') as HTMLElement
    cancel.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.clearHistory).not.toHaveBeenCalled()
    const questions = wrapper.findAllComponents({ name: 'ConfirmDialog' })
    expect(questions.every((question) => question.props('open') === false)).toBe(true)
  })

  it('draws the rows of the window alone when the history is long', async () => {
    apiStub.getHistory.mockResolvedValue(
      Array.from({ length: 500 }, (_unused, index) => ({
        ...entry,
        id: `h${index}`,
        query: `SELECT ${index}`,
      })),
    )
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()

    const rows = wrapper.findAll('[data-test="history-entry"]')
    expect(rows.length).toBeGreaterThan(0)
    expect(rows.length).toBeLessThan(50)
    expect(rows[0]?.text()).toContain('SELECT 0')
  })
})

describe('HistoryPanel height', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.getHistory.mockResolvedValue([entry])
  })

  it('follows the height of its body, and works without a watcher of it', async () => {
    // The library draws parts that watch their own size, so each call gets
    // an empty list of entries.
    const callbacks: Array<(entries: unknown[]) => void> = []
    class ObserverStub {
      constructor(callback: (entries: unknown[]) => void) {
        callbacks.push(callback)
      }
      observe(): void {}
      unobserve(): void {}
      disconnect(): void {}
    }
    const held = globalThis.ResizeObserver
    globalThis.ResizeObserver = ObserverStub as unknown as typeof ResizeObserver
    try {
      const wrapper = mountWithPlugins(HistoryPanel)
      await useHistoryStore().load()
      await wrapper.vm.$nextTick()
      const body = wrapper.find('.body').element
      const list = () => wrapper.findComponent({ name: 'VVirtualScroll' })

      Object.defineProperty(body, 'clientHeight', { value: 300, configurable: true })
      callbacks.forEach((callback) => callback([]))
      await wrapper.vm.$nextTick()
      expect(list().props('height')).toBe(300)

      // A height of none says nothing, so the list keeps the height it knows.
      Object.defineProperty(body, 'clientHeight', { value: 0, configurable: true })
      callbacks.forEach((callback) => callback([]))
      await wrapper.vm.$nextTick()
      expect(list().props('height')).toBe(300)
      wrapper.unmount()
    } finally {
      globalThis.ResizeObserver = held
    }

    // A host without the watcher opens the panel all the same. The list of
    // the library needs the watcher, so the history here is empty.
    apiStub.getHistory.mockResolvedValue([])
    // @ts-expect-error the test takes the watcher away from the host.
    delete globalThis.ResizeObserver
    try {
      const wrapper = mountWithPlugins(HistoryPanel)
      expect(wrapper.text()).toContain('No history yet')
      wrapper.unmount()
    } finally {
      globalThis.ResizeObserver = held
    }
  })
})

describe('HistoryPanel states', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.getHistory.mockResolvedValue([])
  })

  it('shows the reason of a failed statement in words', async () => {
    apiStub.getHistory.mockResolvedValue([
      { ...entry, id: 'h2', succeeded: false, error: 'Invalid column name.' },
      { ...entry, id: 'h3', succeeded: false, error: null },
    ])
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()
    const errors = wrapper.findAll('[data-test="history-error"]')
    expect(errors[0]!.text()).toContain('Invalid column name.')
    expect(errors[1]!.text()).toContain('Failed')
    const rows = wrapper.findAll('[data-test="history-entry"]')
    expect(rows[0]!.find('.d-sr-only').text()).toBe('Failed:')
    expect(rows[0]!.find('.query-line').attributes('title')).toContain('Invalid column name.')
    expect(rows[0]!.find('.v-icon').attributes('aria-hidden')).toBe('true')
  })

  it('names a passed statement for a reader', async () => {
    apiStub.getHistory.mockResolvedValue([entry])
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.vm.$nextTick()
    const row = wrapper.find('[data-test="history-entry"]')
    expect(row.find('.d-sr-only').text()).toBe('Succeeded:')
    expect(row.find('.query-line').attributes('title')).toBe('SELECT 1')
  })

  it('shuts the Clear button while the history is empty', () => {
    const wrapper = mountWithPlugins(HistoryPanel)
    expect(wrapper.find('[data-test="clear-history"]').attributes('disabled')).toBeDefined()
  })

  it('says when the filter hides every entry, and clears the filter', async () => {
    apiStub.getHistory.mockResolvedValue([entry])
    const wrapper = mountWithPlugins(HistoryPanel)
    const history = useHistoryStore()
    await history.load()
    history.filter = 'nothing like this'
    await settle()
    await new Promise((resolve) => setTimeout(resolve, 400))
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('No matches')
    await wrapper.find('[data-test="history-clear-filter"]').trigger('click')
    expect(history.filter).toBe('')
  })
})
