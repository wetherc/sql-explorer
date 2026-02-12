// frontend/src/components/__tests__/ConnectionForm.spec.ts
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createTestingPinia } from '@pinia/testing'
import { useConnectionManagerStore, type SavedConnection } from '@/stores/connectionManager'
import ConnectionForm from '../ConnectionForm.vue'
import { VCard, VCardTitle, VCardText, VForm, VTextField, VSelect, VCardActions, VSpacer, VBtn } from 'vuetify/components'
import { createVuetify } from 'vuetify'

// Mock uuid
vi.mock('uuid', () => ({
  v1: vi.fn(),
  v3: vi.fn(),
  v4: vi.fn(),
  v5: vi.fn(),
  NIL: vi.fn(),
}))

const vuetify = createVuetify()

// Helper to mount the component with Vuetify
function mountComponent(options?: any): VueWrapper<any> {
  return mount(ConnectionForm, {
    global: {
      plugins: [
        vuetify, // Use the created Vuetify instance
        createTestingPinia({
          createSpy: vi.fn,
          initialState: options?.initialState || {}, // Allow passing initialState, but default to empty
        }),
      ],
      components: {
        VCard, VCardTitle, VCardText, VForm, VTextField, VSelect, VCardActions, VSpacer, VBtn
      },
    },
    ...options,
  })
}

describe('ConnectionForm.vue', () => {
  let store: ReturnType<typeof useConnectionManagerStore>

  beforeEach(() => {
    setActivePinia(createPinia())
    store = useConnectionManagerStore()
    vi.clearAllMocks()
    vi.spyOn(store, 'saveConnection')
    vi.spyOn(store, 'cancelConnectionForm')
  })

  it('renders correctly for a new connection', () => {
    const wrapper = mountComponent()
    expect(wrapper.findComponent(VCardTitle).text()).toBe('New Connection')
    expect(wrapper.findComponent(VForm).exists()).toBe(true)
    expect(wrapper.findAllComponents(VTextField).length).toBe(6) // Name, Host, Port, User, Database (and one for the hidden input for port as number)
    expect(wrapper.findComponent(VSelect).exists()).toBe(true) // DbType
  })

  it('populates form fields when editing an existing connection', async () => {
    const mockConnection: SavedConnection = {
      id: 'test-id',
      name: 'Edit Me',
      dbType: 'Postgres',
      host: 'localhost',
      port: 5432,
      user: 'testuser',
      database: 'testdb',
    }
    
    // Set initial state directly on the store before mounting
    store.editingConnection = mockConnection;
    store.showConnectionForm = true; // This will trigger the watch in the component

    const wrapper = mountComponent()
    await wrapper.vm.$nextTick() // Wait for Vue to react to state change

    expect(wrapper.findComponent(VCardTitle).text()).toBe('Edit Connection')
    expect(wrapper.findComponent(VTextField, { props: { label: 'Connection Name' } }).props('modelValue')).toBe('Edit Me')
    expect(wrapper.findComponent(VSelect, { props: { label: 'Database Type' } }).props('modelValue')).toBe('Postgres')
    expect(wrapper.findComponent(VTextField, { props: { label: 'Host' } }).props('modelValue')).toBe('localhost')
    expect(wrapper.findComponent(VTextField, { props: { label: 'Port' } }).props('modelValue')).toBe(5432)
    expect(wrapper.findComponent(VTextField, { props: { label: 'User' } }).props('modelValue')).toBe('testuser')
    expect(wrapper.findComponent(VTextField, { props: { label: 'Database' } }).props('modelValue')).toBe('testdb')
  })

  it('validates required fields', async () => {
    const wrapper = mountComponent()
    const formComponent = wrapper.findComponent(VForm)
    const saveButton = wrapper.findComponent('[data-test="save-button"]')
    expect(saveButton.exists()).toBe(true)

    formComponent.vm.validate = vi.fn().mockResolvedValue({ valid: false })
    await saveButton.trigger('click')

    expect(store.saveConnection).not.toHaveBeenCalled()
  })

  it('calls saveConnection on valid form submission', async () => {
    const wrapper = mountComponent()
    const formComponent = wrapper.findComponent(VForm)
    const saveButton = wrapper.findComponent('[data-test="save-button"]')
    expect(saveButton.exists()).toBe(true)

    const newConn: SavedConnection = {
      id: '',
      name: 'Valid Conn',
      dbType: 'Mysql',
      host: '127.0.0.1',
      port: 3306,
      user: 'root',
      database: 'mydb',
    }
    wrapper.vm.connection = newConn
    
    formComponent.vm.validate = vi.fn().mockResolvedValue({ valid: true })
    await saveButton.trigger('click')

    expect(store.saveConnection).toHaveBeenCalledWith(expect.objectContaining(newConn))
  })

  it('calls cancelConnectionForm on cancel button click', async () => {
    const wrapper = mountComponent()
    const cancelButton = wrapper.findComponent('[data-test="cancel-button"]')
    expect(cancelButton.exists()).toBe(true)
    await cancelButton.trigger('click')
    expect(store.cancelConnectionForm).toHaveBeenCalled()
  })
})
