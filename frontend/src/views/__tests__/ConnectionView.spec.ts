import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import PrimeVue from 'primevue/config'
import ToastService from 'primevue/toastservice'
import ConnectionView from '../ConnectionView.vue'
import { useConnectionStore } from '@/stores/connection'
import * as connBuilder from '@/utils/connectionStringBuilder' // Corrected import
import { AuthType, DbType } from '@/types/savedConnection'
import { useToast } from 'primevue/usetoast'

// Mock dependencies
vi.mock('@tauri-apps/api/tauri', () => ({ invoke: vi.fn() }))
vi.mock('@/stores/savedConnections', () => ({
  useSavedConnectionsStore: vi.fn(() => ({ connections: [], fetchConnections: vi.fn() })),
}))
vi.mock('primevue/usetoast')

describe('ConnectionView.vue', () => {
  const mockToastAdd = vi.fn()

  const mountComponent = () => {
    setActivePinia(createPinia())
    vi.mocked(useToast).mockReturnValue({ add: mockToastAdd } as any)
    
    // JSDOM doesn't implement matchMedia, which some PrimeVue components need
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
    
    return mount(ConnectionView, {
      global: {
        plugins: [PrimeVue, ToastService],
        stubs: {
          SavedConnections: true,
          SaveConnectionDialog: true,
          // Stub PrimeVue form components to make them easier to test
          InputText: {
            props: ['id', 'modelValue'],
            template: '<input :id="id" :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
          },
          Dropdown: {
            props: ['id', 'modelValue', 'options'],
            template: `
              <select :id="id" :value="modelValue" @change="$emit('update:modelValue', $event.target.value)">
                <option v-for="option in options" :key="option.value" :value="option.value">{{ option.label }}</option>
              </select>
            `,
          },
          InputNumber: {
            props: ['id', 'modelValue'],
            template: '<input type="number" :id="id" :value="modelValue" @input="$emit(\'update:modelValue\', parseFloat($event.target.value))" />',
          },
          Checkbox: {
            props: ['id', 'modelValue', 'binary'],
            template: '<input type="checkbox" :id="id" :checked="modelValue" @change="$emit(\'update:modelValue\', $event.target.checked)" />',
          },
          Password: { // Password is a special case as it renders an InputText internally
            props: ['id', 'modelValue'],
            template: '<input type="password" :id="id" :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
          },
          FloatLabel: { template: '<div><slot></slot></div>' },
          Card: { template: '<div><slot name="title"></slot><slot name="content"></slot><slot name="footer"></slot></div>' },
          Button: { props: ['label'], template: '<button type="button">{{label}}</button>' }, // Added type="button"
        },
      },
    })
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders base form fields correctly', () => {
    const wrapper = mountComponent()
    expect(wrapper.find('#db-type').exists()).toBe(true)
    expect(wrapper.find('#server').exists()).toBe(true)
    expect(wrapper.find('#auth-type').exists()).toBe(true)
  })

  it('shows username and password fields for SQL auth', () => {
    const wrapper = mountComponent()
    expect(wrapper.find('#username').exists()).toBe(true)
    expect(wrapper.find('#password').exists()).toBe(true)
  })

  it('calls the connection string builder and store action on submit', async () => {
    const wrapper = mountComponent()
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')
    const builderSpy = vi
      .spyOn(connBuilder, 'buildConnectionString')
      .mockReturnValue('fake-connection-string')

    await wrapper.find('#server').setValue('test-server')
    await wrapper.find('#username').setValue('test-user')
    await wrapper.find('#password').setValue('test-pass')
    await wrapper.find('form').trigger('submit.prevent')

    expect(builderSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        dbType: DbType.Mssql,
        server: 'test-server',
        username: 'test-user',
        password: 'test-pass'
      })
    )
    expect(connectSpy).toHaveBeenCalledWith('fake-connection-string', DbType.Mssql)
  })

  it('displays an error if the builder throws an error', async () => {
    vi.spyOn(connBuilder, 'buildConnectionString').mockImplementation(() => {
      throw new Error('Invalid server name')
    })
    const wrapper = mountComponent()
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')
    
    await wrapper.find('form').trigger('submit.prevent')
    
    expect(connectSpy).not.toHaveBeenCalled()
    expect(connectionStore.errorMessage).toBe('Invalid server name')
  })

  it('changes port to 3306 for MySQL', async () => {
    const wrapper = mountComponent()
    const dbTypeDropdown = wrapper.find('#db-type')
    await dbTypeDropdown.setValue(DbType.Mysql)
    
    const portInput = wrapper.find('#port')
    expect((portInput.element as HTMLInputElement).value).toBe('3306')
  })

  it('hides MS SQL specific options for MySQL', async () => {
    const wrapper = mountComponent()
    const dbTypeDropdown = wrapper.find('#db-type')
    await dbTypeDropdown.setValue(DbType.Mysql)

    expect(wrapper.find('#trustServerCertificate').exists()).toBe(false)
    
    const authTypeDropdown = wrapper.find('#auth-type')
    expect((authTypeDropdown.element as HTMLSelectElement).length).toBe(1)
    expect((authTypeDropdown.element as HTMLSelectElement).options[0]!.text).toBe('SQL Authentication')
  })

  it('changes port to 5432 for PostgreSQL', async () => {
    const wrapper = mountComponent()
    const dbTypeDropdown = wrapper.find('#db-type')
    await dbTypeDropdown.setValue(DbType.Postgres)
    
    const portInput = wrapper.find('#port')
    expect((portInput.element as HTMLInputElement).value).toBe('5432')
  })
})
