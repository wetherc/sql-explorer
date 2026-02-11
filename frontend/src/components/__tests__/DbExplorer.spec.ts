import { describe, it, expect, vi, beforeEach, type Mock, type MockInstance } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import PrimeVue from 'primevue/config'
import DbExplorer from '../DbExplorer.vue'
import { useExplorerStore } from '../../stores/explorer'
import { useTabsStore } from '../../stores/tabs'
import { useConnectionStore } from '@/stores/connection'
import { ref, h } from 'vue'

// Mock stores and dependencies
vi.mock('@/stores/explorer', () => ({ useExplorerStore: vi.fn() }))
vi.mock('@/stores/tabs', () => ({ useTabsStore: vi.fn() }))
vi.mock('@/stores/connection', () => ({ useConnectionStore: vi.fn() }))

vi.mock('primevue/tree', () => ({
  default: {
    props: ['value', 'loading'],
    render(props) {
      const nodesToRender = props.value || [];
      return h('div', { class: 'p-tree-stub' }, [
        nodesToRender.length > 0
          ? nodesToRender.map(node => h('div', { class: 'p-treenode', key: node.key }, [
              h('span', {
                class: 'p-treenode-label',
                onContextmenu: (event) => this.$emit('node-contextmenu', { originalEvent: event, node: node }),
              }, node.label)
            ]))
          : h('div', 'No nodes'),
      ]);
    }
  },
}));

vi.mock('primevue/contextmenu', () => ({
  default: {
    props: ['model'],
    render() {
      return h('div', { class: 'p-contextmenu-stub' });
    },
    methods: {
      show: vi.fn(),
      hide: vi.fn(),
    },
  },
}));

describe('DbExplorer.vue', () => {
  let explorerStoreMock: ReturnType<typeof useExplorerStore>
  let tabsStoreMock: ReturnType<typeof useTabsStore>
  let connectionStoreMock: ReturnType<typeof useConnectionStore>

  const createExplorerStoreMock = (databases: any[] = []) => {
    const store = {
      databases: ref(databases), // Make databases reactive
      loading: ref(false),
      error: ref(null),
      fetchDatabases: vi.fn().mockImplementation(async () => {
        store.loading.value = true;
        await new Promise(resolve => setTimeout(resolve, 1)); // Simulate async
        store.loading.value = false;
        return databases; // Directly return the databases
      }),
    };
    return store;
  };

  const mountComponent = (initialExplorerState: any = {}) => {
    setActivePinia(createPinia())
    
    explorerStoreMock = createExplorerStoreMock(initialExplorerState.databases) as any;
    (useExplorerStore as any).mockReturnValue(explorerStoreMock) // TODO: Fix TS2707 Generic type 'MockInstance<T>'

    tabsStoreMock = { addTab: vi.fn() } as any
    ;(useTabsStore as any).mockReturnValue(tabsStoreMock) // TODO: Fix TS2707 Generic type 'MockInstance<T>'
    
    connectionStoreMock = { dbType: 'Mssql' } as any
    ;(useConnectionStore as any).mockReturnValue(connectionStoreMock) // TODO: Fix TS2707 Generic type 'MockInstance<T>'

    // Mock JSDOM environment for PrimeVue
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: vi.fn().mockImplementation(query => ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    })

    return mount(DbExplorer, {
      global: {
        plugins: [PrimeVue],
        // stubs: {
        //   // Simplified Tree stub
        //   Tree: {
        //     props: ['value'],
        //     template: `
        //       <div class="p-tree-stub">
        //         <div v-for="node in value" :key="node.key" class="p-treenode">
        //           <span class="p-treenode-label" @contextmenu="$emit('node-contextmenu', { originalEvent: $event, node: node })">{{ node.label }}</span>
        //         </div>
        //         <slot name="empty"></slot>
        //       </div>
        //     `,
        //   },
        //   ContextMenu: {
        //     props: ['model'],
        //     template: `
        //       <div class="p-contextmenu-stub">
        //         <div v-for="(item, i) in model" :key="i" @click="item.command()">{{ item.label }}</div>
        //       </div>
        //     `,
        //     methods: {
        //       show: vi.fn(), // Mock the show method
        //       hide: vi.fn(),
        //     },
        //   },
        //   Skeleton: true,
        // },
      },
    })
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  // it('fetches databases on mount and renders them', async () => {
  //   const wrapper = mountComponent({ databases: [{ key: 'db1', label: 'db1' }] })
  //   await flushPromises()
  //   await wrapper.vm.$nextTick() // Ensure reactivity is flushed and component re-rendered

  //   expect(explorerStoreMock.fetchDatabases).toHaveBeenCalledTimes(1)
  //   const treeComponent = wrapper.findComponent({ name: 'Tree' })
  //   expect(treeComponent.exists()).toBe(true)
  // })

  // it('shows context menu for a database node', async () => {
  //   const wrapper = mountComponent({ databases: [{ key: 'db1', label: 'db1' }] })
  //   await wrapper.vm.$nextTick()
  //   await wrapper.vm.$nextTick()
    
  //   const treeNode = wrapper.find('.p-treenode-label')
  //   await treeNode.trigger('contextmenu', { node: { key: 'db1', label: 'db1' } }) // Pass a mock node

  //   const contextMenu = wrapper.findComponent({ name: 'ContextMenu' })
  //   expect(contextMenu.props('model')).toEqual([
  //     expect.objectContaining({ label: 'New Query' }),
  //   ])
  //   expect(contextMenu.vm.show).toHaveBeenCalledWith(expect.any(MouseEvent)) // Verify show was called
  // })

  // it('calls addTab when "New Query" is selected from context menu', async () => {
  //   const wrapper = mountComponent({ databases: [{ key: 'db1', label: 'db1' }] })
  //   await wrapper.vm.$nextTick()
  //   await wrapper.vm.$nextTick()

  //   const treeNode = wrapper.find('.p-treenode-label')
  //   await treeNode.trigger('contextmenu', { node: { key: 'db1', label: 'db1' } })

  //   const contextMenu = wrapper.findComponent({ name: 'ContextMenu' })
  //   const newQueryItem = contextMenu.find('div') // Assuming the first div is the "New Query" item
  //   await newQueryItem.trigger('click')

  //   expect(tabsStoreMock.addTab).toHaveBeenCalledWith('')
  //   expect(contextMenu.vm.hide).toHaveBeenCalledTimes(1) // Verify hide was called
  // })
})