// frontend/src/stores/__tests__/explorer.spec.ts
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { createPinia } from 'pinia' // Changed from setActivePinia, createPinia
import { createTestingPinia } from '@pinia/testing' // Import createTestingPinia
import { useExplorerStore, type ExplorerNode } from '../explorer'
import { useConnectionStore } from '../connection' // Keep import to access its state via testing pinia
import { DbType } from '@/types/db'

// Mock the Tauri API
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}))

describe('explorer store', () => {
  let store: ReturnType<typeof useExplorerStore>
  let connectionStore: ReturnType<typeof useConnectionStore> // Use the actual store from testing pinia

  beforeEach(() => {
    vi.clearAllMocks()

    // Use createTestingPinia to set up Pinia for testing
    // and provide initial state for the connection store
    createTestingPinia({
      createPinia,
      initialState: {
        connection: {
          isConnected: true,
          dbType: DbType.Mssql,
        },
      },
      stubActions: false, // Don't stub actions, we want to test them
    })

    store = useExplorerStore()
    connectionStore = useConnectionStore() // Get the actual connection store from testing pinia
    store.clearExplorer() // Ensure store is reset before each test
  })

  it('initial state is correct', () => {
    expect(store.nodes).toEqual([])
    expect(store.loading).toBe(false)
    expect(store.error).toBeNull()
  })

  it('clearExplorer resets the state', () => {
    store.nodes = [{ key: '1', label: 'db1', data: { type: 'database' } }]
    store.error = 'Some error'
    store.loading = true

    store.clearExplorer()

    expect(store.nodes).toEqual([])
    expect(store.error).toBeNull()
    expect(store.loading).toBe(false)
  })

  describe('fetchDatabases', () => {
    it('fetches databases successfully for Mssql', async () => {
      const { invoke } = await import('@tauri-apps/api/tauri')
      const mockDatabases = [{ name: 'master' }, { name: 'tempdb' }]
      vi.mocked(invoke).mockResolvedValue(mockDatabases)

      await store.fetchDatabases()

      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
      expect(store.nodes.length).toBe(2)
      expect(store.nodes[0]).toEqual({
        key: 'master',
        label: 'master',
        icon: 'mdi-database',
        children: [],
        data: { type: 'database', db: 'master' },
      })
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('list_databases')
    })

    it('sets error message on fetch failure', async () => {
      const { invoke } = await import('@tauri-apps/api/tauri')
      const errorMsg = 'Failed to fetch databases'
      vi.mocked(invoke).mockRejectedValue(errorMsg)

      await store.fetchDatabases()

      expect(store.loading).toBe(false)
      expect(store.error).toBe(errorMsg)
      expect(store.nodes).toEqual([])
    })

    it('does nothing if not connected', async () => {
      const { invoke } = await import('@tauri-apps/api/tauri')
      connectionStore.isConnected = false

      await store.fetchDatabases()

      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
      expect(store.nodes).toEqual([])
      expect(vi.mocked(invoke)).not.toHaveBeenCalled()
    })
  })

  describe('expandNode', () => {
    it('does nothing if not connected', async () => {
      const { invoke } = await import('@tauri-apps/api/tauri')
      connectionStore.isConnected = false
      const node: ExplorerNode = { key: 'db1', label: 'db1', data: { type: 'database', db: 'db1' }, children: [] }

      await store.expandNode(node)

      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
      expect(vi.mocked(invoke)).not.toHaveBeenCalled()
    })

    it('does nothing if children are already loaded', async () => {
      const { invoke } = await import('@tauri-apps/api/tauri')
      const node: ExplorerNode = {
        key: 'db1',
        label: 'db1',
        data: { type: 'database', db: 'db1' },
        children: [{ key: 'schema1', label: 'schema1', data: { type: 'schema' } }],
      }

      await store.expandNode(node)

      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
      expect(vi.mocked(invoke)).not.toHaveBeenCalled()
    })

    it('expands database node and fetches schemas for Mssql', async () => {
      const { invoke } = await import('@tauri-apps/api/tauri')
      const node: ExplorerNode = { key: 'db1', label: 'db1', data: { type: 'database', db: 'db1' }, children: [] }
      const mockSchemas = [{ name: 'dbo' }, { name: 'guest' }]
      vi.mocked(invoke).mockResolvedValue(mockSchemas)

      await store.expandNode(node)

      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
      expect(node.children?.length).toBe(2)
      expect(node.children?.[0]).toEqual({
        key: 'db1-dbo',
        label: 'dbo',
        icon: 'mdi-folder-outline',
        children: [],
        data: { type: 'schema', db: 'db1', schema: 'dbo' },
      })
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('list_schemas', { database: 'db1' })
    })

    it('expands schema node and fetches tables for Mssql', async () => {
      const { invoke } = await import('@tauri-apps/api/tauri')
      const node: ExplorerNode = {
        key: 'db1-dbo',
        label: 'dbo',
        data: { type: 'schema', db: 'db1', schema: 'dbo' },
        children: [],
      }
      const mockTables = [{ name: 'Users' }, { name: 'Products' }]
      vi.mocked(invoke).mockResolvedValue(mockTables)

      await store.expandNode(node)

      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
      expect(node.children?.length).toBe(2)
      expect(node.children?.[0]).toEqual({
        key: 'db1-dbo-Users',
        label: 'Users',
        icon: 'mdi-table',
        data: { type: 'table', db: 'db1', schema: 'dbo', table: 'Users' },
      })
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('list_tables', { database: 'db1', schemaName: 'dbo' })
    })

    it('expands database node and fetches tables for Mysql', async () => {
      const { invoke } = await import('@tauri-apps/api/tauri')
      connectionStore.dbType = DbType.Mysql
      const node: ExplorerNode = { key: 'db1', label: 'db1', data: { type: 'database', db: 'db1' }, children: [] }
      const mockTables = [{ name: 'users' }, { name: 'orders' }]
      vi.mocked(invoke).mockResolvedValue(mockTables)

      await store.expandNode(node)

      expect(store.loading).toBe(false)
      expect(store.error).toBeNull()
      expect(node.children?.length).toBe(2)
      expect(node.children?.[0]).toEqual({
        key: 'db1-users',
        label: 'users',
        icon: 'mdi-table',
        data: { type: 'table', db: 'db1', schema: 'db1', table: 'users' },
      })
      expect(vi.mocked(invoke)).toHaveBeenCalledWith('list_tables', { database: 'db1' })
    })

    it('sets error message on expand failure', async () => {
      const { invoke } = await import('@tauri-apps/api/tauri')
      const node: ExplorerNode = { key: 'db1', label: 'db1', data: { type: 'database', db: 'db1' }, children: [] }
      const errorMsg = 'Failed to expand node'
      vi.mocked(invoke).mockRejectedValue(errorMsg)

      await store.expandNode(node)

      expect(store.loading).toBe(false)
      expect(store.error).toBe(errorMsg)
      expect(node.children?.length).toBe(1) // Error node added
      expect(node.children?.[0]?.label).toBe('Error')
    })
  })
})
