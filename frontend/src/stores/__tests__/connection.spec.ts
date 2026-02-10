import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useConnectionStore } from '../connection'
import { DbType } from '@/types/savedConnection'
import { invoke } from '@tauri-apps/api/tauri'

// Mock the tauri invoke function
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: vi.fn()
}))

describe('connection store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  it('has correct initial state', () => {
    const store = useConnectionStore()
    expect(store.isConnected).toBe(false)
    expect(store.isConnecting).toBe(false)
    expect(store.errorMessage).toBe('')
    expect(store.dbType).toBeNull()
  })

  it('connect action successfully connects', async () => {
    const store = useConnectionStore()
    const connectionString = 'server=test'
    const dbType = DbType.Mssql

    vi.mocked(invoke).mockResolvedValue(undefined)

    await store.connect(connectionString, dbType)

    expect(store.isConnecting).toBe(false)
    expect(store.isConnected).toBe(true)
    expect(store.dbType).toBe(dbType)
    expect(store.errorMessage).toBe('')
    expect(invoke).toHaveBeenCalledWith('connect', { connectionString, dbType })
  })

  it('connect action handles connection failure', async () => {
    const store = useConnectionStore()
    const connectionString = 'server=fail'
    const dbType = DbType.Mysql
    const error = 'Connection failed'

    vi.mocked(invoke).mockRejectedValue(error)

    await store.connect(connectionString, dbType)

    expect(store.isConnecting).toBe(false)
    expect(store.isConnected).toBe(false)
    expect(store.dbType).toBeNull()
    expect(store.errorMessage).toBe(error)
  })

  it('disconnect action resets state', () => {
    const store = useConnectionStore()
    // Set a connected state first
    store.isConnected = true
    store.dbType = DbType.Mssql

    store.disconnect()

    expect(store.isConnected).toBe(false)
    expect(store.dbType).toBeNull()
  })
})
