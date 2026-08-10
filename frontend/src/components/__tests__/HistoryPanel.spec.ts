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

const savedQuery = {
  id: 'q1',
  name: 'Daily count',
  query: 'SELECT COUNT(*) FROM orders',
  connectionId: 'c1',
  folder: null,
  updatedAt: '2026-08-10T00:00:00Z',
}

describe('HistoryPanel', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.getHistory.mockResolvedValue([])
    apiStub.getSavedQueries.mockResolvedValue([])
  })

  it('says so when nothing has run and nothing is saved', async () => {
    const wrapper = mountWithPlugins(HistoryPanel)
    expect(wrapper.text()).toContain('No statement has run yet')

    await wrapper.find('[data-test="mode-saved"]').trigger('click')
    expect(wrapper.text()).toContain('No statement is saved yet')
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
    // The history is emptied only once the user answers the question.
    expect(apiStub.clearHistory).not.toHaveBeenCalled()

    const confirm = document.querySelector('[data-test="confirm-accept"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(apiStub.clearHistory).toHaveBeenCalled()
  })

  it('lists the saved statements and opens one under its own name', async () => {
    apiStub.getSavedQueries.mockResolvedValue([savedQuery])
    const wrapper = mountWithPlugins(HistoryPanel)
    await useConnectionsStore().load()
    await useHistoryStore().load()
    await wrapper.find('[data-test="mode-saved"]').trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="saved-entry"]').trigger('click')
    const tabs = useTabsStore()
    expect(tabs.tabs[0]?.title).toBe('Daily count')
    expect(tabs.tabs[0]?.connectionId).toBe('c1')
  })

  it('opens a saved statement on the selected connection when it names none', async () => {
    apiStub.getSavedQueries.mockResolvedValue([{ ...savedQuery, connectionId: null }])
    const wrapper = mountWithPlugins(HistoryPanel)
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')
    await useHistoryStore().load()
    await wrapper.find('[data-test="mode-saved"]').trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="saved-entry"]').trigger('click')
    expect(useTabsStore().tabs[0]?.connectionId).toBe('c1')
  })

  it('removes a saved statement', async () => {
    apiStub.getSavedQueries.mockResolvedValue([savedQuery])
    apiStub.deleteSavedQuery.mockResolvedValue(undefined)
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.find('[data-test="mode-saved"]').trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="delete-saved"]').trigger('click')
    await settle()
    expect(apiStub.deleteSavedQuery).not.toHaveBeenCalled()

    const confirm = document.querySelector('[data-test="confirm-accept"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(apiStub.deleteSavedQuery).toHaveBeenCalledWith('q1')
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

  it('keeps a saved statement when the question is refused', async () => {
    apiStub.getSavedQueries.mockResolvedValue([savedQuery])
    const wrapper = mountWithPlugins(HistoryPanel)
    await useHistoryStore().load()
    await wrapper.find('[data-test="mode-saved"]').trigger('click')
    await wrapper.vm.$nextTick()

    await wrapper.find('[data-test="delete-saved"]').trigger('click')
    await settle()
    expect(document.body.textContent).toContain('Daily count')
    const cancel = document.querySelector('[data-test="confirm-cancel"]') as HTMLElement
    cancel.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(apiStub.deleteSavedQuery).not.toHaveBeenCalled()
  })
})
