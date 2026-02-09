import { setActivePinia, createPinia } from 'pinia'
import { useSavedConnectionsStore } from '../savedConnections'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/tauri'
import { AuthType, type SavedConnection } from '@/types/savedConnection'

// Mock the tauri invoke function
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn()
}))

describe('Saved Connections Store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    // Reset mocks before each test
    vi.mocked(invoke).mockReset()
  })

  it('fetches connections and stores them', async () => {
    const store = useSavedConnectionsStore()
    const mockConnections: SavedConnection[] = [
      { name: 'Test 1', server: 'srv1', database: 'db1', auth_type: AuthType.Sql, user: 'sa' },
      { name: 'Test 2', server: 'srv2', database: 'db2', auth_type: AuthType.Integrated }
    ]
    vi.mocked(invoke).mockResolvedValue(mockConnections)

    await store.fetchConnections()

    expect(store.isLoading).toBe(false)
    expect(store.connections).toEqual(mockConnections)
    expect(invoke).toHaveBeenCalledWith('list_connections')
  })

  it('saves a connection and refreshes the list', async () => {
    const store = useSavedConnectionsStore()
    const newConnection: SavedConnection = {
      name: 'New Test',
      server: 'new_srv',
      database: 'new_db',
      auth_type: AuthType.Sql,
      user: 'user'
    }
    const password = 'password123'

    // Mock return for the refresh call
    vi.mocked(invoke).mockResolvedValue([newConnection])

    await store.saveConnection(newConnection, password)

    expect(invoke).toHaveBeenCalledWith('save_connection', {
      name: newConnection.name,
      server: newConnection.server,
      database: newConnection.database,
      authType: newConnection.auth_type,
      user: newConnection.user,
      password: password
    })
    expect(invoke).toHaveBeenCalledWith('list_connections')
    expect(store.connections).toEqual([newConnection])
  })

  it('deletes a connection and refreshes the list', async () => {
    const store = useSavedConnectionsStore()
    const connectionName = 'Test 1'

    // Mock the list_connections invoke to return an empty array after deletion
    vi.mocked(invoke).mockResolvedValue([])

    await store.deleteConnection(connectionName)

    expect(invoke).toHaveBeenCalledWith('delete_connection', { name: connectionName })
    expect(invoke).toHaveBeenCalledWith('list_connections')
    expect(store.connections).toEqual([])
  })

  it('gets a password for a connection', async () => {
    const store = useSavedConnectionsStore()
    const connectionName = 'Test 1'
    const mockPassword = 'supersecretpassword'

    vi.mocked(invoke).mockResolvedValue(mockPassword)

    const password = await store.getPassword(connectionName)

    expect(password).toBe(mockPassword)
    expect(invoke).toHaveBeenCalledWith('get_connection_password', { name: connectionName })
  })

  it('handles errors during fetch', async () => {
    const store = useSavedConnectionsStore()
    const errorMessage = 'Failed to fetch'
    vi.mocked(invoke).mockRejectedValue(errorMessage)

    await store.fetchConnections()

    expect(store.errorMessage).toBe(errorMessage)
    expect(store.connections).toEqual([])
  })
})
