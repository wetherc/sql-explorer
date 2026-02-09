import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ConnectionView from '../ConnectionView.vue'
import { useSavedConnectionsStore } from '@/stores/savedConnections'
import { AuthType } from '@/types/savedConnection'

// Mock child components
const SavedConnections = {
  template: '<div class="mock-saved-connections" @select="$emit('select', mockConnection)"></div>',
  props: [],
  data() {
    return {
      mockConnection: {
        name: 'My Test Connection',
        server: 'test.server.com',
        database: 'testdb',
        auth_type: AuthType.Sql,
        user: 'tester'
      }
    }
  }
}

const SaveConnectionDialog = {
  template: '<div v-if="show" class="mock-save-dialog"></div>',
  props: ['show', 'connection', 'password']
}

vi.mock('@/stores/savedConnections', () => ({
  useSavedConnectionsStore: vi.fn(() => ({
    connections: [],
    fetchConnections: vi.fn(),
    saveConnection: vi.fn(),
    deleteConnection: vi.fn(),
    getPassword: vi.fn()
  }))
}))

describe('ConnectionView.vue - Saved Connections', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
  })

  it('shows the "Save Connection" button', () => {
    const wrapper = mount(ConnectionView, {
      global: {
        stubs: {
          SavedConnections: true,
          SaveConnectionDialog: true
        }
      }
    })
    const saveButton = wrapper.findAll('button').find(b => b.text() === 'Save Connection')
    expect(saveButton).toBeDefined()
  })

  it('opens the SaveConnectionDialog when "Save Connection" is clicked', async () => {
    const wrapper = mount(ConnectionView, {
      global: {
        stubs: {
          SavedConnections: true,
          SaveConnectionDialog // Stubbing to check props
        }
      }
    })

    const saveButton = wrapper.findAll('button').find(b => b.text() === 'Save Connection')!
    await saveButton.trigger('click')

    const dialog = wrapper.findComponent(SaveConnectionDialog)
    expect(dialog.props('show')).toBe(true)
  })

  it('populates the form when a saved connection is selected', async () => {
    const wrapper = mount(ConnectionView, {
      global: {
        stubs: {
          SavedConnections, // Using mock component that emits
          SaveConnectionDialog: true
        }
      }
    })

    // Find the mock component and trigger the event
    const savedConnectionsComponent = wrapper.findComponent(SavedConnections)
    await savedConnectionsComponent.vm.$emit('select', savedConnectionsComponent.vm.mockConnection)


    // Check that the form fields were updated
    const serverInput = wrapper.find('input#server')
    expect((serverInput.element as HTMLInputElement).value).toBe('test.server.com')

    const dbInput = wrapper.find('input#database')
    expect((dbInput.element as HTMLInputElement).value).toBe('testdb')

    const authSelect = wrapper.find('select#auth-type')
    expect((authSelect.element as HTMLSelectElement).value).toBe(AuthType.Sql)

    const userInput = wrapper.find('input#username')
    expect((userInput.element as HTMLInputElement).value).toBe('tester')

    // Password should be cleared for saved connections
    const passInput = wrapper.find('input#password')
    expect((passInput.element as HTMLInputElement).value).toBe('')
  })
})
