import { beforeEach, describe, expect, it, vi } from 'vitest'
import { makeApiStub, connectionFixture, infoFixture } from '../../stores/__tests__/helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn(), open: vi.fn() }))

const QueryTabs = (await import('@/components/QueryTabs.vue')).default
const { mountWithPlugins, settle } = await import('./mount')
const { useConnectionsStore } = await import('@/stores/connections')
const { useTabsStore } = await import('@/stores/tabs')

describe('QueryTabs', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
  })

  it('points at the connections when none is open', () => {
    const wrapper = mountWithPlugins(QueryTabs)
    expect(wrapper.text()).toContain('No open tabs')
    expect(wrapper.text()).toContain('Open a connection first')
    expect(wrapper.find('[data-test="empty-open-connections"]').exists()).toBe(true)
  })

  it('asks the layout to show the connections', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    await wrapper.find('[data-test="empty-open-connections"]').trigger('click')
    expect(wrapper.emitted('open-connections')).toHaveLength(1)
  })

  it('offers a new tab when a connection is open', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    await useConnectionsStore().load()
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Open a tab to write a statement')

    await wrapper.find('[data-test="empty-new-tab"]').trigger('click')
    expect(useTabsStore().tabs).toHaveLength(1)
  })

  it('opens a tab from the button on the strip', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    await wrapper.find('[data-test="new-tab"]').trigger('click')
    expect(useTabsStore().tabs).toHaveLength(1)
  })

  it('draws one tab for each open statement and marks a changed one', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    tabs.add({ query: 'SELECT 1' })
    const second = tabs.add({ query: 'SELECT 2' })
    tabs.setQuery(second.id, 'SELECT 3')
    await settle()

    expect(wrapper.findAll('[data-test="query-tab"]')).toHaveLength(2)
    expect(wrapper.findAll('.dirty-mark')).toHaveLength(1)
  })

  it('closes a tab from its own button', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    tabs.add()
    await settle()

    await wrapper.find('[data-test="close-tab"]').trigger('click')
    expect(tabs.tabs).toHaveLength(0)
  })

  it('closes the tab that holds the focus when the Delete key arrives', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    tabs.add()
    await settle()

    await wrapper.find('[data-test="query-tab"]').trigger('keydown', { key: 'Delete' })
    expect(tabs.tabs).toHaveLength(0)
  })

  it('speaks the change of a tab that a reader cannot see', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    const tab = tabs.add({ query: 'SELECT 1' })
    tabs.setQuery(tab.id, 'SELECT 2')
    await settle()

    expect(wrapper.find('.app-visually-hidden').text()).toBe(', has changes')
    // The mark itself stays out of the reading, so the state is said once.
    expect(wrapper.find('.dirty-mark').attributes('aria-hidden')).toBe('true')
  })

  it('moves to the tab the user chose', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    const first = tabs.add()
    tabs.add()
    await settle()

    await wrapper.findComponent({ name: 'VTabs' }).vm.$emit('update:modelValue', first.id)
    expect(tabs.activeTabId).toBe(first.id)
  })
})
