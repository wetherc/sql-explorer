import { describe, it, expect, vi, beforeEach, type Mock, type MockInstance } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import PrimeVue from 'primevue/config'
import DbExplorer from '../DbExplorer.vue'
import { useExplorerStore } from '../../stores/explorer'
import { useTabsStore } from '../../stores/tabs'
import { useConnectionStore } from '@/stores/connection'
import { ref, computed, h } from 'vue'

// Mock stores and dependencies
vi.mock('@/stores/explorer', () => ({ useExplorerStore: vi.fn() }))
vi.mock('@/stores/tabs', () => ({ useTabsStore: vi.fn() }))
vi.mock('@/stores/connection', () => ({ useConnectionStore: vi.fn() }))

// Revert global vi.mock for primevue/tree to a basic mock or remove if not used elsewhere


vi.mock('primevue/contextmenu', () => ({
  default: {
    name: 'ContextMenu', // Added name property
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

// Tree Stub Component Definition
const TreeStub = {
  name: 'Tree',
  props: ['value', 'selectionMode', 'selectionKeys'],
  emits: ['node-expand', 'node-contextmenu', 'node-select', 'update:selectionKeys'],
  template: `
    <div class="p-tree-stub">
      <template v-if="value && value.length > 0">
        <div v-for="node in value" :key="node.key" class="p-treenode">
          <span
            class="p-treenode-label"
            @contextmenu="$emit('node-contextmenu', { originalEvent: $event, node: node })"
            @click="$emit('node-select', { node: node })"
          >
            {{ node.label }}
          </span>
          <div v-if="node.children && node.children.length > 0" class="p-treenode-children">
            <div v-for="child in node.children" :key="child.key" class="p-treenode">
              <span
                class="p-treenode-label"
                @contextmenu="$emit('node-contextmenu', { originalEvent: $event, node: child })"
                @click="$emit('node-select', { node: child })"
              >
                {{ child.label }}
              </span>
              <div v-if="child.children && child.children.length > 0" class="p-treenode-children">
                <div v-for="grandchild in child.children" :key="grandchild.key" class="p-treenode">
                  <span
                    class="p-treenode-label"
                    @contextmenu="$emit('node-contextmenu', { originalEvent: $event, node: grandchild })"
                    @click="$emit('node-select', { node: grandchild })"
                  >
                    {{ grandchild.label }}
                  </span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </template>
      <div v-else>No nodes</div>
    </div>
  `,
};

describe('DbExplorer.vue', () => {
  let explorerStoreMock: ReturnType<typeof useExplorerStore>
  let tabsStoreMock: ReturnType<typeof useTabsStore>
  let connectionStoreMock: ReturnType<typeof useConnectionStore>

  const createExplorerStoreMock = (mockDatabases: any[] = [], mockSchemas: any[] = [], mockTables: any[] = []) => {
    const store = {
      databases: ref(mockDatabases),
      schemas: ref(mockSchemas),
      tables: ref(mockTables),
      loading: ref(false),
      error: ref(null),
      nodes: ref<TreeNode[]>([]), // Initialize nodes as a ref
      fetchDatabases: vi.fn().mockImplementation(() => { // Removed async
        store.loading.value = true;
        store.loading.value = false;
        
        // Manually construct nodes based on mock data and set it to the reactive nodes ref
        store.nodes.value = mockDatabases.map(db => ({
          key: db.name,
          label: db.name,
          icon: 'pi pi-fw pi-database',
          data: { type: 'database', db: db.name },
          children: mockSchemas.filter(schema => schema.database_name === db.name).map(schema => ({
            key: `${db.name}-${schema.name}`,
            label: schema.name,
            icon: 'pi pi-fw pi-folder',
            data: { type: 'schema', db: db.name, schema: schema.name },
            children: mockTables.filter(table => table.database_name === db.name && table.schema_name === schema.name).map(table => ({
              key: `${db.name}-${schema.name}-${table.name}`,
              label: table.name,
              icon: 'pi pi-fw pi-table',
              data: { type: 'table', db: db.name, schema: schema.name, table: table.name }
            }))
          }))
        }));
      }),
    };
    return store;
  };

  const mountComponent = async (initialExplorerState: any = {}) => {
    setActivePinia(createPinia())
    
    explorerStoreMock = createExplorerStoreMock(
      initialExplorerState.databases,
      initialExplorerState.schemas,
      initialExplorerState.tables
    ) as any;
    (useExplorerStore as Mock).mockReturnValue(explorerStoreMock)

    tabsStoreMock = { addTab: vi.fn() } as any
    ;(useTabsStore as Mock).mockReturnValue(tabsStoreMock)
    
    connectionStoreMock = { dbType: 'Mssql' } as any
    ;(useConnectionStore as Mock).mockReturnValue(connectionStoreMock)

    // Manually trigger fetchDatabases to populate nodes in the mock store
    // This is crucial because DbExplorer.vue calls fetchDatabases in onMounted
    // await explorerStoreMock.fetchDatabases(); // Removed: DbExplorer.vue calls this itself in onMounted
    // await flushPromises(); // Removed: fetchDatabases is now synchronous

    return mount(DbExplorer, {
      global: {
        plugins: [PrimeVue],
        stubs: {
          QueryView: true,
          Tree: TreeStub,
          ContextMenu: {
            name: 'ContextMenu',
            props: ['model'],
            template: `
              <div class="p-contextmenu-stub">
                <div v-for="(item, i) in model" :key="i" @click="item.command()">{{ item.label }}</div>
              </div>
            `,
            methods: {
              show: vi.fn(),
              hide: vi.fn(),
            },
          },
          Skeleton: true,
        },
      },
    })
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('fetches databases on mount and renders them', async () => {
    const mockDatabases = [{ name: 'TestDB1' }]
    const wrapper = await mountComponent({ databases: mockDatabases })
    await wrapper.vm.$nextTick() // Ensure reactivity is flushed and component re-rendered

    expect(explorerStoreMock.fetchDatabases).toHaveBeenCalledTimes(1)
    const treeComponent = wrapper.findComponent(TreeStub)
    expect(treeComponent.exists()).toBe(true)
  })

  it('shows context menu for a database node', async () => {
    const mockDatabases = [{ name: 'TestDB1' }]
    const mockSchemas = [{ name: 'Schema1', database_name: 'TestDB1' }]
    const mockTables = [{ name: 'TableA', database_name: 'TestDB1', schema_name: 'Schema1' }]
    const wrapper = await mountComponent({ 
      databases: mockDatabases,
      schemas: mockSchemas,
      tables: mockTables
    })
    await wrapper.vm.$nextTick()
    
    // Find the tree node label and simulate a right-click
    const treeNode = wrapper.find('.p-treenode-label')
    await treeNode.trigger('contextmenu', { 
      originalEvent: new MouseEvent('contextmenu'), 
      node: { key: 'TestDB1', label: 'TestDB1', data: { type: 'database', db: 'TestDB1' } } 
    }) 

    const contextMenu = wrapper.findComponent({ name: 'ContextMenu' })
    expect(contextMenu.props('model')).toEqual([
      expect.objectContaining({ label: 'New Query' }),
    ])
    // Assuming the show method is mocked as vi.fn()
    expect(contextMenu.vm.show).toHaveBeenCalledWith(expect.any(MouseEvent))
  })

  it('calls addTab when "New Query" is selected from context menu', async () => {
    const mockDatabases = [{ name: 'TestDB1' }]
    const mockSchemas = [{ name: 'Schema1', database_name: 'TestDB1' }]
    const mockTables = [{ name: 'TableA', database_name: 'TestDB1', schema_name: 'Schema1' }]
    const wrapper = await mountComponent({ 
      databases: mockDatabases,
      schemas: mockSchemas,
      tables: mockTables
    })
    await wrapper.vm.$nextTick()

    const treeNode = wrapper.find('.p-treenode-label')
    await treeNode.trigger('contextmenu', { 
      originalEvent: new MouseEvent('contextmenu'), 
      node: { key: 'TestDB1', label: 'TestDB1', data: { type: 'database', db: 'TestDB1' } } 
    })

    const contextMenu = wrapper.findComponent({ name: 'ContextMenu' })
    // Simulate clicking the "New Query" item in the context menu
    // The stub for ContextMenu renders items as divs and calls item.command() on click
    const newQueryItem = contextMenu.findAll('div')[0] // Assuming the first div is "New Query"
    await newQueryItem.trigger('click')

    expect(tabsStoreMock.addTab).toHaveBeenCalledWith()
  })

  it('calls addTab with correct query when a table node is clicked', async () => {
    const mockDatabases = [{ name: 'TestDB1' }]
    const mockSchemas = [{ name: 'Schema1', database_name: 'TestDB1' }]
    const mockTables = [{ name: 'TableA', database_name: 'TestDB1', schema_name: 'Schema1' }]
    const wrapper = await mountComponent({
      databases: mockDatabases,
      schemas: mockSchemas,
      tables: mockTables
    })
    await wrapper.vm.$nextTick()

    // Simulate clicking on the table node
    // The TreeStub renders .p-treenode-label for each node
    const tableNodeLabel = wrapper.findAll('.p-treenode-label')[2] // Assuming TableA is the third label in the flat list
    await tableNodeLabel.trigger('click')

    const expectedQuery = 'SELECT TOP (1000) * FROM [Schema1].[TableA]'
    expect(tabsStoreMock.addTab).toHaveBeenCalledWith(expectedQuery)
  })
})