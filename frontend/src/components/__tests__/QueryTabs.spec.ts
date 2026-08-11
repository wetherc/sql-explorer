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
    // The view of a tab reads the names of the parameters as it opens.
    apiStub.queryParameters.mockResolvedValue([])
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

  it('mounts a view for the tab the user opens and not for the others', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    const first = tabs.add()
    tabs.add()
    await settle()

    expect(wrapper.findAll('.query-view')).toHaveLength(1)
    tabs.activate(first.id)
    await settle()
    expect(wrapper.findAll('.query-view')).toHaveLength(2)
  })

  it('keeps a view for the five tabs the user opened last', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    const opened = Array.from({ length: 7 }, () => tabs.add())
    for (const tab of opened) {
      tabs.activate(tab.id)
      await settle()
    }

    expect(wrapper.findAll('.query-view')).toHaveLength(5)
  })

  it('gives the place of a closed tab to another tab', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    const opened = Array.from({ length: 6 }, () => tabs.add())
    for (const tab of opened) {
      tabs.activate(tab.id)
      await settle()
    }
    expect(wrapper.findAll('.query-view')).toHaveLength(5)

    // The tab that stands open closes, and the tab that lost its view early
    // takes the place that the closed tab held.
    tabs.close(opened[5]!.id)
    tabs.activate(opened[0]!.id)
    await settle()
    expect(wrapper.findAll('.query-view')).toHaveLength(5)
  })
})

describe('QueryTabs renaming a tab', () => {
  beforeEach(() => {
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    // The view of a tab reads the names of the parameters as it opens.
    apiStub.queryParameters.mockResolvedValue([])
  })

  /** Mounts the view with one tab and opens the edit of its name. */
  async function mountEditing() {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    const tab = tabs.add()
    await settle()

    await wrapper.find('[data-test="query-tab"]').trigger('dblclick')
    await settle()
    return { wrapper, tabs, tab }
  }

  it('writes the name that the edit holds', async () => {
    const { wrapper, tabs, tab } = await mountEditing()
    const field = wrapper.find('[data-test="tab-title-field"]')
    expect(field.exists()).toBe(true)
    expect((field.element as HTMLInputElement).value).toBe(tab.title)
    // The edit takes the pointer, so the user writes at once.
    expect(document.activeElement).toBe(field.element)

    await field.setValue('Sales report')
    await field.trigger('keydown', { key: 'Enter' })
    await settle()

    expect(tabs.tabs[0]?.title).toBe('Sales report')
    expect(wrapper.find('[data-test="tab-title-field"]').exists()).toBe(false)
  })

  it('keeps the pointer events of the edit away from the tab', async () => {
    const { wrapper, tabs } = await mountEditing()
    const field = wrapper.find('[data-test="tab-title-field"]')

    await field.trigger('mousedown')
    await field.trigger('click')
    await field.trigger('dblclick')
    await settle()

    // The edit stands open, and the click did not close it or start another.
    expect(wrapper.find('[data-test="tab-title-field"]').exists()).toBe(true)
    expect(tabs.tabs).toHaveLength(1)
  })

  it('writes the name when the field loses the focus', async () => {
    const { wrapper, tabs } = await mountEditing()

    await wrapper.find('[data-test="tab-title-field"]').setValue('From the blur')
    await wrapper.find('[data-test="tab-title-field"]').trigger('blur')
    await settle()

    expect(tabs.tabs[0]?.title).toBe('From the blur')
  })

  it('keeps the name that the tab holds when the edit is cancelled', async () => {
    const { wrapper, tabs, tab } = await mountEditing()

    await wrapper.find('[data-test="tab-title-field"]').setValue('Thrown away')
    await wrapper.find('[data-test="tab-title-field"]').trigger('keydown', { key: 'Escape' })
    await settle()

    expect(tabs.tabs[0]?.title).toBe(tab.title)
    expect(wrapper.find('[data-test="tab-title-field"]').exists()).toBe(false)
  })

  it('keeps the name that the tab holds for an empty text', async () => {
    const { wrapper, tabs, tab } = await mountEditing()

    await wrapper.find('[data-test="tab-title-field"]').setValue('   ')
    await wrapper.find('[data-test="tab-title-field"]').trigger('keydown', { key: 'Enter' })
    await settle()

    expect(tabs.tabs[0]?.title).toBe(tab.title)
  })

  it('writes nothing when the field goes after a cancel', async () => {
    const { wrapper, tabs, tab } = await mountEditing()
    const field = wrapper.find('[data-test="tab-title-field"]')

    await field.setValue('Thrown away')
    await field.trigger('keydown', { key: 'Escape' })
    // The field goes with the cancel, and its blur reaches the view after.
    await field.trigger('blur')
    await settle()

    expect(tabs.tabs[0]?.title).toBe(tab.title)
  })

  it('starts an edit of the tab that stands open', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    tabs.add()
    await settle()
    ;(wrapper.vm as unknown as { renameActiveTab: () => void }).renameActiveTab()
    await settle()

    expect(wrapper.find('[data-test="tab-title-field"]').exists()).toBe(true)
  })

  it('starts no edit while no tab is open', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    await settle()
    ;(wrapper.vm as unknown as { renameActiveTab: () => void }).renameActiveTab()
    await settle()

    expect(wrapper.find('[data-test="tab-title-field"]').exists()).toBe(false)
  })
})

describe('QueryTabs asking before it loses work', () => {
  beforeEach(() => {
    apiStub.queryParameters.mockResolvedValue([])
  })

  it('closes a tab that holds no change without a question', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    tabs.add()
    await settle()

    await wrapper.find('[data-test="close-tab"]').trigger('click')

    expect(tabs.tabs).toHaveLength(0)
    expect(document.body.textContent).not.toContain('Close this tab?')
  })

  it('asks before it closes a tab whose changes are not saved', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    const tab = tabs.add({ query: 'SELECT 1' })
    tabs.setQuery(tab.id, 'SELECT 2')
    await settle()

    await wrapper.find('[data-test="close-tab"]').trigger('click')
    await settle()
    expect(tabs.tabs).toHaveLength(1)
    expect(document.body.textContent).toContain('Close this tab?')

    const confirm = document.querySelector('[data-test="confirm-accept"]') as HTMLElement
    confirm.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()
    expect(tabs.tabs).toHaveLength(0)
  })

  it('keeps the tab when the question is refused', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    const tab = tabs.add({ query: 'SELECT 1' })
    tabs.setQuery(tab.id, 'SELECT 2')
    await settle()

    await wrapper.find('[data-test="close-tab"]').trigger('click')
    await settle()
    const cancel = document.querySelector('[data-test="confirm-cancel"]') as HTMLElement
    cancel.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle()

    expect(tabs.tabs).toHaveLength(1)
  })

  it('asks the same question when the Delete key closes a changed tab', async () => {
    const wrapper = mountWithPlugins(QueryTabs)
    const tabs = useTabsStore()
    const tab = tabs.add({ query: 'SELECT 1' })
    tabs.setQuery(tab.id, 'SELECT 2')
    await settle()

    await wrapper.find('[data-test="query-tab"]').trigger('keydown', { key: 'Delete' })
    await settle()

    expect(tabs.tabs).toHaveLength(1)
    expect(document.body.textContent).toContain('Close this tab?')
  })
})
