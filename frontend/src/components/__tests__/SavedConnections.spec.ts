import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import SavedConnections from '../SavedConnections.vue'
import { useSavedConnectionsStore } from '@/stores/savedConnections'
import { AuthType, DbType } from '@/types/savedConnection'

vi.mock('@/stores/savedConnections', () => ({
  useSavedConnectionsStore: vi.fn(),
}))

describe('SavedConnections.vue', () => {
  let savedConnectionsStoreMock: ReturnType<typeof useSavedConnectionsStore>

  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()

    savedConnectionsStoreMock = {
      connections: [],
      isLoading: false,
      errorMessage: '',
      fetchConnections: vi.fn(),
      deleteConnection: vi.fn(),
      saveConnection: vi.fn(),
      getPassword: vi.fn(),
    } as any
    ;(useSavedConnectionsStore as Mock).mockReturnValue(savedConnectionsStoreMock)
  })

  it('fetches connections on mount', () => {
    mount(SavedConnections)
    expect(savedConnectionsStoreMock.fetchConnections).toHaveBeenCalledTimes(1)
  })

  it('displays a loading message when isLoading is true', () => {
    savedConnectionsStoreMock.isLoading = true
    const wrapper = mount(SavedConnections)
    expect(wrapper.find('div').text()).toContain('Loading...')
  })

  it('displays an error message when errorMessage is set', () => {
    savedConnectionsStoreMock.errorMessage = 'Failed to fetch'
    const wrapper = mount(SavedConnections)
    expect(wrapper.find('.error-message').text()).toBe('Failed to fetch')
  })

  it('renders a list of saved connections', () => {
    savedConnectionsStoreMock.connections = [
      { name: 'Conn 1', db_type: DbType.Mssql, server: 's1', database: 'd1', auth_type: AuthType.Sql, user: 'u1' },
      { name: 'Conn 2', db_type: DbType.Mysql, server: 's2', database: 'd2', auth_type: AuthType.Sql, user: 'u2' },
    ]
    const wrapper = mount(SavedConnections)
    const items = wrapper.findAll('li')
    expect(items.length).toBe(2)
    expect(items[0].text()).toContain('Conn 1')
    expect(items[0].text()).toContain('(s1)')
    expect(items[1].text()).toContain('Conn 2')
    expect(items[1].text()).toContain('(s2)')
  })

  it('emits "select" event when a connection is clicked', async () => {
    const connection = { name: 'Conn 1', db_type: DbType.Mssql, server: 's1', database: 'd1', auth_type: AuthType.Sql, user: 'u1' }
    savedConnectionsStoreMock.connections = [connection]
    const wrapper = mount(SavedConnections)

    await wrapper.find('.connection-name').trigger('click')

    expect(wrapper.emitted().select).toBeTruthy()
    expect(wrapper.emitted().select![0]).toEqual([connection])
  })

  it('calls deleteConnection when delete button is clicked', async () => {
    // Mock window.confirm
    window.confirm = vi.fn(() => true)

    const connectionName = 'Conn 1'
    savedConnectionsStoreMock.connections = [
      { name: connectionName, db_type: DbType.Mssql, server: 's1', database: 'd1', auth_type: AuthType.Sql, user: 'u1' },
    ]
    const wrapper = mount(SavedConnections)

    await wrapper.find('.delete-btn').trigger('click')

    expect(window.confirm).toHaveBeenCalledWith('Are you sure you want to delete the connection "Conn 1"?')
    expect(savedConnectionsStoreMock.deleteConnection).toHaveBeenCalledWith(connectionName)
  })
})
