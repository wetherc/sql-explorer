// frontend/src/components/__tests__/ConnectionManager.spec.ts
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createTestingPinia } from '@pinia/testing'
import { useConnectionManagerStore, type SavedConnection } from '@/stores/connectionManager'
import ConnectionManager from '../ConnectionManager.vue'
import ConnectionForm from '../ConnectionForm.vue'
import { VBtn, VList, VListItem, VContainer, VRow, VCol, VDialog } from 'vuetify/components'
import { createVuetify } from 'vuetify'
import { invoke } from '@tauri-apps/api/tauri'

// Mock the Tauri API invoke function
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}))

// Mock uuid, although not directly used in ConnectionManager.vue, it's used in its store
vi.mock('uuid', () => ({
  v1: vi.fn(),
  v3: vi.fn(),
  v4: vi.fn(),
  v5: vi.fn(),
  NIL: vi.fn(),
}))

const vuetify = createVuetify()

// Helper to mount the component with Vuetify and Pinia
function mountComponent(options?: any): VueWrapper<any> {
  const defaultInitialState = {
    connectionManager: {
      connections: [],
      loading: false,
      error: null,
      showConnectionForm: false,
      editingConnection: null,
    },
  }
  const initialState = options?.initialState || defaultInitialState

  return mount(ConnectionManager, {
    global: {
      plugins: [
        vuetify,
        createTestingPinia({
          createSpy: vi.fn,
          initialState: initialState,
        }),
      ],
      components: {
        VBtn, VList, VListItem, VContainer, VRow, VCol, VDialog, ConnectionForm
      },
    },
    ...options,
  })
}

describe('ConnectionManager.vue', () => {
  let store: ReturnType<typeof useConnectionManagerStore>

  beforeEach(() => {
    setActivePinia(createPinia())
    store = useConnectionManagerStore()
    vi.clearAllMocks()
    vi.spyOn(store, 'fetchConnections') // Don't mockResolvedValue here
    vi.spyOn(store, 'newConnection')
    vi.spyOn(store, 'editConnection')
    vi.spyOn(store, 'deleteConnection')
    vi.mocked(invoke).mockResolvedValue(undefined) // Mock invoke for connect
  })

  it('renders a new connection button', () => {
    const wrapper = mountComponent()
    expect(wrapper.findComponent(VBtn).text()).toContain('New Connection')
  })

  it('calls fetchConnections on mounted', () => {
    mountComponent()
    expect(store.fetchConnections).toHaveBeenCalled()
  })

  it('displays loading state', async () => {
    const wrapper = mountComponent({
      initialState: { connectionManager: { loading: true } },
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Loading connections...')
  })

  it('displays error message', async () => {
    const wrapper = mountComponent({
      initialState: { connectionManager: { error: 'Test Error' } },
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Test Error')
  })

  it('displays a list of connections', async () => {
    const mockConnections: SavedConnection[] = [
      { id: '1', name: 'Conn1', dbType: 'Mssql', host: 'h1' },
      { id: '2', name: 'Conn2', dbType: 'Mysql', host: 'h2' },
    ]
    const wrapper = mountComponent({
      initialState: { connectionManager: { connections: mockConnections } },
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents(VListItem).length).toBe(2)
    expect(wrapper.findAllComponents(VListItem)[0].attributes('title')).toBe('Conn1')
    expect(wrapper.findAllComponents(VListItem)[1].attributes('title')).toBe('Conn2')
  })

  it('calls newConnection when "New Connection" button is clicked', async () => {
    const wrapper = mountComponent()
    await wrapper.findComponent(VBtn, { text: 'New Connection' }).trigger('click')
    expect(store.newConnection).toHaveBeenCalled()
  })

  it('calls editConnection when edit button is clicked for a connection', async () => {
    const mockConnections: SavedConnection[] = [{ id: '1', name: 'Conn1', dbType: 'Mssql', host: 'h1' }]
    const wrapper = mountComponent({
      initialState: { connectionManager: { connections: mockConnections } },
    })
    await wrapper.vm.$nextTick()
    // Find the VListItem and then find the specific button within it
    const listItem = wrapper.findComponent(VListItem)
    const editButton = listItem.findComponent('[icon="mdi-pencil"]') // More robust way to find
    expect(editButton.exists()).toBe(true)
    await editButton.trigger('click')
    expect(store.editConnection).toHaveBeenCalledWith(mockConnections[0])
  })

  it('calls deleteConnection when delete button is clicked for a connection', async () => {
    const mockConnections: SavedConnection[] = [{ id: '1', name: 'Conn1', dbType: 'Mssql', host: 'h1' }]
    const wrapper = mountComponent({
      initialState: { connectionManager: { connections: mockConnections } },
    })
    await wrapper.vm.$nextTick()
    const listItem = wrapper.findComponent(VListItem)
    const deleteButton = listItem.findComponent('[icon="mdi-delete"]') // More robust way to find
    expect(deleteButton.exists()).toBe(true)
    await deleteButton.trigger('click')
    expect(store.deleteConnection).toHaveBeenCalledWith('1')
  })

  it('calls connect when connect button is clicked for a connection (Mssql)', async () => {
    const mockConnections: SavedConnection[] = [{
      id: '1', name: 'Conn1', dbType: 'Mssql', host: 'localhost', port: 1433, user: 'sa', database: 'master'
    }]
    const wrapper = mountComponent({
      initialState: { connectionManager: { connections: mockConnections } },
    })
    await wrapper.vm.$nextTick()
    const listItem = wrapper.findComponent(VListItem)
    const connectButton = listItem.findComponent('[icon="mdi-connection"]') // More robust way to find
    expect(connectButton.exists()).toBe(true)
    await connectButton.trigger('click')
    expect(invoke).toHaveBeenCalledWith('connect', {
      connectionString: 'server=localhost;port=1433;user=sa;database=master;TrustServerCertificate=true;',
      dbType: 'Mssql',
    })
  })

  it('calls connect when connect button is clicked for a connection (Mysql)', async () => {
    const mockConnections: SavedConnection[] = [{
      id: '2', name: 'Conn2', dbType: 'Mysql', host: 'localhost', port: 3306, user: 'root', database: 'testdb'
    }]
    const wrapper = mountComponent({
      initialState: { connectionManager: { connections: mockConnections } },
    })
    await wrapper.vm.$nextTick()
    const listItem = wrapper.findComponent(VListItem)
    const connectButton = listItem.findComponent('[icon="mdi-connection"]')
    expect(connectButton.exists()).toBe(true)
    await connectButton.trigger('click')
    expect(invoke).toHaveBeenCalledWith('connect', {
      connectionString: 'mysql://root@localhost:3306/testdb',
      dbType: 'Mysql',
    })
  })

  it('calls connect when connect button is clicked for a connection (Postgres)', async () => {
    const mockConnections: SavedConnection[] = [{
      id: '3', name: 'Conn3', dbType: 'Postgres', host: 'localhost', port: 5432, user: 'postgres', database: 'mydb'
    }]
    const wrapper = mountComponent({
      initialState: { connectionManager: { connections: mockConnections } },
    })
    await wrapper.vm.$nextTick()
    const listItem = wrapper.findComponent(VListItem)
    const connectButton = listItem.findComponent('[icon="mdi-connection"]')
    expect(connectButton.exists()).toBe(true)
    await connectButton.trigger('click')
    expect(invoke).toHaveBeenCalledWith('connect', {
      connectionString: 'postgresql://postgres@localhost:5432/mydb',
      dbType: 'Postgres',
    })
  })

  it('shows the ConnectionForm dialog when showConnectionForm is true', async () => {
    const wrapper = mountComponent({
      initialState: { connectionManager: { showConnectionForm: true } },
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent(VDialog).props('modelValue')).toBe(true)
    expect(wrapper.findComponent(ConnectionForm).exists()).toBe(true)
  })

  it('hides the ConnectionForm dialog when showConnectionForm is false', async () => {
    const wrapper = mountComponent({
      initialState: { connectionManager: { showConnectionForm: false } },
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent(VDialog).props('modelValue')).toBe(false)
  })
})