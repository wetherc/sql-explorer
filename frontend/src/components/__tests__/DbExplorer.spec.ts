import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { default as DbExplorer } from '../DbExplorer.vue'
import { useExplorerStore } from '../../stores/explorer'
import { useTabsStore } from '../../stores/tabs'

// Mock stores
vi.mock('@/stores/explorer', () => ({
  useExplorerStore: vi.fn(),
}))
vi.mock('@/stores/tabs', () => ({
  useTabsStore: vi.fn(),
}))

describe('DbExplorer.vue', () => {
  let explorerStoreMock: ReturnType<typeof useExplorerStore>
  let tabsStoreMock: ReturnType<typeof useTabsStore>

  beforeEach(() => {
    setActivePinia(createPinia())

    const tempPinia = createPinia();

    const tempExplorerStore = useExplorerStore(tempPinia);
    explorerStoreMock = {
      databases: [{ name: 'db1' }],
      schemas: [{ name: 'dbo' }],
      tables: [{ TABLE_NAME: 'table1' }],
      columns: [],
      loading: false,
      error: null,
      fetchDatabases: vi.fn().mockResolvedValue(true),
      fetchSchemas: vi.fn().mockResolvedValue(true),
      fetchTables: vi.fn().mockResolvedValue(true),
      fetchColumns: vi.fn().mockResolvedValue(true),
      $state: tempExplorerStore.$state,
      $patch: tempExplorerStore.$patch,
      $reset: tempExplorerStore.$reset,
      $subscribe: tempExplorerStore.$subscribe,
      $onAction: tempExplorerStore.$onAction,
      $id: tempExplorerStore.$id,
      $dispose: tempExplorerStore.$dispose,
    } as unknown as ReturnType<typeof useExplorerStore>; // Cast to bypass strict type checking for complex Pinia mocks
    (useExplorerStore as Mock).mockReturnValue(explorerStoreMock)

    const tempTabsStore = useTabsStore(tempPinia);
    tabsStoreMock = {
      tabs: [],
      activeTabId: null,
      activeTab: null,
      addTab: vi.fn(),
      closeTab: vi.fn(),
      setActiveTab: vi.fn(),
      $state: tempTabsStore.$state,
      $patch: tempTabsStore.$patch,
      $reset: tempTabsStore.$reset,
      $subscribe: tempTabsStore.$subscribe,
      $onAction: tempTabsStore.$onAction,
      $id: tempTabsStore.$id,
      $dispose: tempTabsStore.$dispose,
    } as unknown as ReturnType<typeof useTabsStore>; // Cast to bypass strict type checking
    (useTabsStore as Mock).mockReturnValue(tabsStoreMock)
  })

  it('fetches databases on mount and renders them', async () => {
    const wrapper = mount(DbExplorer)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()
    expect(explorerStoreMock.fetchDatabases).toHaveBeenCalledTimes(1)
    expect(wrapper.text()).toContain('db1')
  })

  it('shows context menu for a database node', async () => {
    const wrapper = mount(DbExplorer)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    await wrapper.find('.tree-item').trigger('contextmenu')

    const contextMenu = wrapper.findComponent({ name: 'ContextMenu' })
    expect(contextMenu.exists()).toBe(true)
    expect(contextMenu.props('items')).toEqual([
      { label: 'New Query', action: 'new-query' },
    ])
  })

  it('calls addTab when "New Query" is selected from context menu', async () => {
    const wrapper = mount(DbExplorer)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    await wrapper.find('.tree-item').trigger('contextmenu')
    const contextMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await contextMenu.vm.$emit('select', 'new-query')

    expect(tabsStoreMock.addTab).toHaveBeenCalledWith('')
  })
})
