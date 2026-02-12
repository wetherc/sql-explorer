// frontend/src/stores/__tests__/connection.spec.ts
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useConnectionStore } from '../connection'
import { DbType } from '@/types/db'

// Mock the Tauri API
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn(),
}))

describe('connection store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    // Reset mocks before each test
    vi.clearAllMocks()
  })

  it('initial state is correct', () => {
    const store = useConnectionStore()
    expect(store.isConnected).toBe(false)
    expect(store.isConnecting).toBe(false)
    expect(store.errorMessage).toBeNull()
    expect(store.dbType).toBeNull()
  })

  it('connects successfully', async () => {
    const store = useConnectionStore()
    const { invoke } = await import('@tauri-apps/api/tauri')
    ;(invoke as vi.Mock).mockResolvedValue(undefined) // Mock successful invoke

    const connectionString = 'test_conn_string'
    const dbType = DbType.Mssql

    await store.connect(connectionString, dbType)

    expect(store.isConnecting).toBe(false)
    expect(store.isConnected).toBe(true)
    expect(store.errorMessage).toBeNull()
    expect(store.dbType).toBe(dbType)
    expect(invoke).toHaveBeenCalledWith('connect', { connectionString, dbType })
  })

  it('fails to connect and sets error message', async () => {
    const store = useConnectionStore()
    const { invoke } = await import('@tauri-apps/api/tauri')
    const errorMsg = 'Failed to connect for some reason'
    ;(invoke as vi.Mock).mockRejectedValue(errorMsg) // Mock failed invoke

    const connectionString = 'bad_conn_string'
    const dbType = DbType.Postgres

    await store.connect(connectionString, dbType)

    expect(store.isConnecting).toBe(false)
    expect(store.isConnected).toBe(false)
    expect(store.errorMessage).toBe(errorMsg)
    expect(store.dbType).toBeNull()
    expect(invoke).toHaveBeenCalledWith('connect', { connectionString, dbType })
  })

  it('disconnects successfully', () => {
    const store = useConnectionStore()
    store.isConnected = true
    store.dbType = DbType.Mysql

    store.disconnect()

    expect(store.isConnected).toBe(false)
    expect(store.dbType).toBeNull()
  })
})
