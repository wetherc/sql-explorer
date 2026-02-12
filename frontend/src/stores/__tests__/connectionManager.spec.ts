// frontend/src/stores/__tests__/connectionManager.spec.ts
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useConnectionManagerStore, type SavedConnection } from '../connectionManager'
import { v4 as uuidv4 } from 'uuid'
import { invoke } from '@tauri-apps/api/tauri'

// Mock the Tauri API invoke function
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}))

// Mock uuid
vi.mock('uuid', () => ({
  v1: vi.fn(),
  v3: vi.fn(),
  v4: vi.fn(),
  v5: vi.fn(),
  NIL: vi.fn(),
}))

describe('connectionManager store', () => {
  let store: ReturnType<typeof useConnectionManagerStore>

  beforeEach(() => {
    setActivePinia(createPinia())
    store = useConnectionManagerStore()
    vi.clearAllMocks()
    vi.mocked(uuidv4).mockReturnValue('mock-uuid')
  })

  it('initial state is correct', () => {
    expect(store.connections).toEqual([])
    expect(store.loading).toBe(false)
    expect(store.error).toBeNull()
    expect(store.showConnectionForm).toBe(false)
    expect(store.editingConnection).toBeNull()
  })

  describe('fetchConnections', () => {
    it('fetches connections successfully', async () => {
      const mockConnections: SavedConnection[] = [
        { id: '1', name: 'Conn1', dbType: 'Mssql', host: 'h1' },
        { id: '2', name: 'Conn2', dbType: 'Mysql', host: 'h2' },
      ]
      vi.mocked(invoke).mockResolvedValue(mockConnections)

      await store.fetchConnections()

      expect(invoke).toHaveBeenCalledWith('get_connections')
      expect(store.connections).toEqual(mockConnections)
      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
    })

    it('sets error message on fetch failure', async () => {
      const errorMsg = 'Failed to fetch'
      vi.mocked(invoke).mockRejectedValue(errorMsg)

      await store.fetchConnections()

      expect(invoke).toHaveBeenCalledWith('get_connections')
      expect(store.connections).toEqual([])
      expect(store.loading).toBe(false)
      expect(store.error).toBe(errorMsg)
    })
  })

  describe('saveConnection', () => {
    it('saves a new connection and refreshes list', async () => {
      const newConnection: SavedConnection = { name: 'NewConn', dbType: 'Postgres', host: 'newhost' }
      const returnedConnections: SavedConnection[] = [{ id: 'mock-uuid', name: 'NewConn', dbType: 'Postgres', host: 'newhost' }]
      vi.mocked(invoke).mockResolvedValue(returnedConnections) // Mock for fetchConnections after save

      await store.saveConnection(newConnection)

      expect(uuidv4).toHaveBeenCalled()
      expect(invoke).toHaveBeenCalledWith('save_connection', {
        connection: { id: 'mock-uuid', name: 'NewConn', dbType: 'Postgres', host: 'newhost' },
      })
      expect(invoke).toHaveBeenCalledWith('get_connections') // Called to refresh
      expect(store.connections).toEqual(returnedConnections)
      expect(store.showConnectionForm).toBe(false)
      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
    })

    it('updates an existing connection and refreshes list', async () => {
      const existingConnection: SavedConnection = { id: '1', name: 'UpdatedConn', dbType: 'Mssql', host: 'updatedhost' }
      const returnedConnections: SavedConnection[] = [existingConnection]
      vi.mocked(invoke).mockResolvedValue(returnedConnections) // Mock for fetchConnections after save

      await store.saveConnection(existingConnection)

      expect(uuidv4).not.toHaveBeenCalled() // No new ID generated
      expect(invoke).toHaveBeenCalledWith('save_connection', { connection: existingConnection })
      expect(invoke).toHaveBeenCalledWith('get_connections') // Called to refresh
      expect(store.connections).toEqual(returnedConnections)
      expect(store.showConnectionForm).toBe(false)
      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
    })

    it('sets error message on save failure', async () => {
      const newConnection: SavedConnection = { name: 'NewConn', dbType: 'Postgres', host: 'newhost' }
      const errorMsg = 'Failed to save'
      vi.mocked(invoke).mockRejectedValue(errorMsg)

      await store.saveConnection(newConnection)

      expect(invoke).toHaveBeenCalledWith('save_connection', expect.any(Object))
      expect(invoke).not.toHaveBeenCalledWith('get_connections') // Not called on failure
      expect(store.loading).toBe(false)
      expect(store.error).toBe(errorMsg)
    })
  })

  describe('deleteConnection', () => {
    it('deletes a connection and refreshes list', async () => {
      const initialConnections: SavedConnection[] = [{ id: '1', name: 'Conn1', dbType: 'Mssql', host: 'h1' }]
      const remainingConnections: SavedConnection[] = []
      store.connections = initialConnections // Pre-fill store for this test
      vi.mocked(invoke).mockResolvedValueOnce(undefined) // Mock for delete_connection
      vi.mocked(invoke).mockResolvedValueOnce(remainingConnections) // Mock for fetchConnections after delete

      await store.deleteConnection('1')

      expect(invoke).toHaveBeenCalledWith('delete_connection', { id: '1' })
      expect(invoke).toHaveBeenCalledWith('get_connections') // Called to refresh
      expect(store.connections).toEqual(remainingConnections)
      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
    })

    it('sets error message on delete failure', async () => {
      const errorMsg = 'Failed to delete'
      vi.mocked(invoke).mockRejectedValue(errorMsg)
      store.connections = [{ id: '1', name: 'Conn1', dbType: 'Mssql', host: 'h1' }] // Pre-fill store

      await store.deleteConnection('1')

      expect(invoke).toHaveBeenCalledWith('delete_connection', { id: '1' })
      expect(invoke).not.toHaveBeenCalledWith('get_connections') // Not called on failure
      expect(store.loading).toBe(false)
      expect(store.error).toBe(errorMsg)
      expect(store.connections).not.toEqual([]) // Should not have been cleared
    })
  })

  describe('form management', () => {
    it('newConnection opens form and clears editing state', () => {
      store.editingConnection = { id: 'any', name: 'Any', dbType: 'Mssql' }
      store.showConnectionForm = false

      store.newConnection()

      expect(store.editingConnection).toBeNull()
      expect(store.showConnectionForm).toBe(true)
    })

    it('editConnection opens form and sets editing state', () => {
      const conn: SavedConnection = { id: '3', name: 'EditMe', dbType: 'Postgres' }
      store.editingConnection = null
      store.showConnectionForm = false

      store.editConnection(conn)

      expect(store.editingConnection).toEqual(conn)
      expect(store.showConnectionForm).toBe(true)
    })

    it('cancelConnectionForm closes form and clears editing state', () => {
      store.editingConnection = { id: 'any', name: 'Any', dbType: 'Mssql' }
      store.showConnectionForm = true

      store.cancelConnectionForm()

      expect(store.editingConnection).toBeNull()
      expect(store.showConnectionForm).toBe(false)
    })
  })
})
