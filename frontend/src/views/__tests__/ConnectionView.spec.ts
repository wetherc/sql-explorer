import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ConnectionView from '../ConnectionView.vue'
import { useConnectionStore } from '@/stores/connection'
import * as connBuilder from '@/utils/connectionStringBuilder'
import { AuthType, DbType } from '@/types/savedConnection'

vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn()
}))

vi.mock('@/stores/savedConnections', () => ({
  useSavedConnectionsStore: vi.fn(() => ({
    connections: [],
    fetchConnections: vi.fn()
  }))
}))

describe('ConnectionView.vue', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
  })

  it('renders base form fields correctly', () => {
    const wrapper = mount(ConnectionView, {
      global: {
        stubs: {
          SavedConnections: true,
          SaveConnectionDialog: true
        }
      }
    })
    expect(wrapper.find('input#server').exists()).toBe(true)
    expect(wrapper.find('input#database').exists()).toBe(true)
    expect(wrapper.find('select#auth-type').exists()).toBe(true)
  })

  it('shows username and password fields for SQL auth', async () => {
    const wrapper = mount(ConnectionView, {
      global: {
        stubs: {
          SavedConnections: true,
          SaveConnectionDialog: true
        }
      }
    })
    await wrapper.find('select#auth-type').setValue(AuthType.Sql)
    expect(wrapper.find('input#username').exists()).toBe(true)
    expect(wrapper.find('input#password').exists()).toBe(true)
  })

  it('calls the connection string builder and store action on submit', async () => {
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')
    const builderSpy = vi
      .spyOn(connBuilder, 'buildConnectionString')
      .mockReturnValue('fake-connection-string')

    const wrapper = mount(ConnectionView, {
      global: {
        stubs: {
          SavedConnections: true,
          SaveConnectionDialog: true
        }
      }
    })

    // Set form values
    await wrapper.find('input#server').setValue('test-server')
    await wrapper.find('input#database').setValue('test-db')
    await wrapper.find('select#auth-type').setValue(AuthType.Sql)
    await wrapper.find('input#username').setValue('test-user')
    await wrapper.find('input#password').setValue('test-pass')

    // Submit
    await wrapper.find('form').trigger('submit.prevent')

    // Check that the builder was called with the correct form data
    expect(builderSpy).toHaveBeenCalledTimes(1)
    expect(builderSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        dbType: DbType.Mssql,
        server: 'test-server',
        database: 'test-db',
        authType: 'sql',
        username: 'test-user',
        password: 'test-pass'
      })
    )

    // Check that the store action was called with the result from the builder
    expect(connectSpy).toHaveBeenCalledTimes(1)
    expect(connectSpy).toHaveBeenCalledWith('fake-connection-string', DbType.Mssql)
  })

  it('displays an error if the builder throws an error', async () => {
    const connectionStore = useConnectionStore()
    const connectSpy = vi.spyOn(connectionStore, 'connect')
    vi.spyOn(connBuilder, 'buildConnectionString').mockImplementation(() => {
      throw new Error('Invalid server name')
    })

    const wrapper = mount(ConnectionView, {
      global: {
        stubs: {
          SavedConnections: true,
          SaveConnectionDialog: true
        }
      }
    })
    await wrapper.find('form').trigger('submit.prevent')

    // Ensure the connect action was NOT called
    expect(connectSpy).not.toHaveBeenCalled()

    // Ensure the error message is displayed
    expect(wrapper.find('.error-message').exists()).toBe(true)
    expect(wrapper.find('.error-message').text()).toBe('Invalid server name')
    expect(connectionStore.errorMessage).toBe('Invalid server name')
  })

  it('changes port to 3306 for MySQL', async () => {
    const wrapper = mount(ConnectionView, {
      global: {
        stubs: {
          SavedConnections: true,
          SaveConnectionDialog: true
        }
      }
    })

    const dbTypeSelect = wrapper.find('select#db-type')
    await dbTypeSelect.setValue(DbType.Mysql)

    const portInput = wrapper.find('input#port')
    expect((portInput.element as HTMLInputElement).value).toBe('3306')
  })

  it('hides MS SQL specific options for MySQL', async () => {
    const wrapper = mount(ConnectionView, {
      global: {
        stubs: {
          SavedConnections: true,
          SaveConnectionDialog: true
        }
      }
    })

    const dbTypeSelect = wrapper.find('select#db-type')
    await dbTypeSelect.setValue(DbType.Mysql)

    // Check that MS SQL specific options are hidden
    expect(wrapper.find('select#encrypt').exists()).toBe(false)
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(false) // Trust server certificate
    const authOptions = wrapper.findAll('select#auth-type option')
    expect(authOptions.length).toBe(1)
    expect(authOptions[0].text()).toBe('SQL Authentication')
  })
})
