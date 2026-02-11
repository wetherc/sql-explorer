import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import PrimeVue from 'primevue/config'
import ToastService from 'primevue/toastservice'
import ConnectionView from '../ConnectionView.vue'
import { useSavedConnectionsStore } from '@/stores/savedConnections'
import { AuthType, DbType } from '@/types/savedConnection'
import { useToast } from 'primevue/usetoast'

// Mock dependencies
vi.mock('@tauri-apps/api/tauri', () => ({ invoke: vi.fn() }))
vi.mock('@/stores/savedConnections')
vi.mock('primevue/usetoast')

describe('ConnectionView.vue - Saved Connections', () => {
  const mockToastAdd = vi.fn()

  const mountComponent = () => {
    setActivePinia(createPinia())
    vi.mocked(useToast).mockReturnValue({ add: mockToastAdd } as any)
    
    vi.mocked(useSavedConnectionsStore).mockReturnValue({
      connections: [
        { name: 'Test Conn', db_type: DbType.Mssql, server: 'test.server', database: 'testdb', auth_type: AuthType.Sql, user: 'testuser' }
      ],
      fetchConnections: vi.fn(),
    } as any)

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
          SaveConnectionDialog: true,
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
          Button: { props: ['label'], template: '<button type="button">{{label}}</button>' },
        },
      },
    })
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows the "Save Connection" button', () => {
    const wrapper = mountComponent()
    const saveButton = wrapper.findAll('button').filter(b => b.text().includes('Save')).at(0)
    expect(saveButton!.exists()).toBe(true) // TODO: Fix TS18048 'saveButton' is possibly 'undefined'.
  })

  // TODO: Fix TypeScript errors in this test block. TS2339
  /*
  it('opens the SaveConnectionDialog when "Save Connection" is clicked', async () => {
    const wrapper = mountComponent()
    const saveButton = wrapper.findAll('button').filter(b => b.text().includes('Save')).at(0)
    await saveButton!.trigger('click')
    expect(wrapper.vm.showSaveDialog.value).toBe(true)
  })
  */

  it('populates the form when a saved connection is selected', async () => {
    const wrapper = mountComponent()
    const savedConnComponent = wrapper.findComponent({ name: 'SavedConnections' })
    await savedConnComponent.vm.$emit('select', {
      name: 'Test Conn',
      db_type: DbType.Mysql,
      server: 'mysql.server',
      database: 'mysqldb',
      auth_type: AuthType.Sql,
      user: 'mysqluser',
    })
    
    const serverInput = wrapper.find('#server')
    expect((serverInput.element as HTMLInputElement).value).toBe('mysql.server')
    
    const dbTypeDropdown = wrapper.find('#db-type')
    expect((dbTypeDropdown.element as HTMLSelectElement).value).toBe(DbType.Mysql)
  })
})
