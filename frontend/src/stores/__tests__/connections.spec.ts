import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { makeApiStub, connectionFixture, infoFixture } from './helpers'

const apiStub = makeApiStub()
vi.mock('@/lib/api', () => ({ api: apiStub, CONNECTION_STATUS_EVENT: 'connection-status' }))

const {
  connectionSubtitle,
  createId,
  defaultPortFor,
  newConnection,
  useConnectionsStore,
  validateConnection,
} = await import('@/stores/connections')
const { useUiStore } = await import('@/stores/ui')
const { ConnectionHealth, DbType } = await import('@/types/api')

describe('newConnection', () => {
  it('starts a network connection on the local host with its default port', () => {
    const connection = newConnection(DbType.Postgres, 'fixed')
    expect(connection.id).toBe('fixed')
    expect(connection.host).toBe('localhost')
    expect(connection.port).toBe(5432)
  })

  it('starts a file or service connection without a host', () => {
    expect(newConnection(DbType.Sqlite).host).toBeNull()
    expect(newConnection(DbType.Athena).host).toBeNull()
  })

  it('starts a MS SQL Server connection by default', () => {
    expect(newConnection().dbType).toBe(DbType.Mssql)
  })
})

describe('defaultPortFor', () => {
  it('gives the port of each engine', () => {
    expect(defaultPortFor(DbType.Mssql)).toBe(1433)
    expect(defaultPortFor(DbType.Mysql)).toBe(3306)
    expect(defaultPortFor(DbType.Postgres)).toBe(5432)
    expect(defaultPortFor(DbType.Sqlite)).toBeNull()
    expect(defaultPortFor(DbType.Athena)).toBeNull()
  })
})

describe('createId', () => {
  it('uses the identifier the host gives', () => {
    expect(createId()).toMatch(/[0-9a-f-]{36}/)
  })

  it('builds its own identifier when the host gives none', () => {
    const original = globalThis.crypto
    Object.defineProperty(globalThis, 'crypto', { configurable: true, value: undefined })
    expect(createId()).toMatch(/^id-/)
    Object.defineProperty(globalThis, 'crypto', { configurable: true, value: original })
  })
})

describe('validateConnection', () => {
  it('accepts a complete record', () => {
    expect(validateConnection(connectionFixture())).toEqual([])
  })

  it('needs a name', () => {
    expect(validateConnection(connectionFixture({ name: ' ' }))).toContain(
      'The connection needs a name.',
    )
  })

  it('needs a host and a port for a network engine', () => {
    expect(validateConnection(connectionFixture({ host: '' }))).toContain(
      'The connection needs a host.',
    )
    expect(validateConnection(connectionFixture({ host: null }))).toContain(
      'The connection needs a host.',
    )
    expect(validateConnection(connectionFixture({ port: null }))).toContain(
      'The port must be a whole number between 1 and 65535.',
    )
    expect(validateConnection(connectionFixture({ port: 0 }))).toHaveLength(1)
    expect(validateConnection(connectionFixture({ port: 70000 }))).toHaveLength(1)
    expect(validateConnection(connectionFixture({ port: 1.5 }))).toHaveLength(1)
  })

  it('needs no host when a connection string is given', () => {
    const connection = connectionFixture({ host: '', port: null })
    connection.options.connectionUrl = 'server=tcp:other,1433'
    expect(validateConnection(connection)).toEqual([])
  })

  it('needs a file for SQLite', () => {
    const connection = connectionFixture({ dbType: DbType.Sqlite })
    expect(validateConnection(connection)).toContain(
      'A SQLite connection needs the path of a file.',
    )
    connection.options.filePath = '/tmp/a.db'
    expect(validateConnection(connection)).toEqual([])
  })

  it('needs a region and a place for the results for Athena', () => {
    const connection = connectionFixture({ dbType: DbType.Athena })
    expect(validateConnection(connection)).toEqual([
      'An Athena connection needs an AWS region.',
      'An Athena connection needs a workgroup or an output location.',
    ])
    connection.options.awsRegion = 'us-east-1'
    connection.options.athenaWorkgroup = 'primary'
    expect(validateConnection(connection)).toEqual([])

    connection.options.athenaWorkgroup = null
    connection.options.athenaOutputLocation = 's3://bucket/'
    expect(validateConnection(connection)).toEqual([])
  })
})

describe('connectionSubtitle', () => {
  it('describes each engine in the terms it uses', () => {
    expect(connectionSubtitle(connectionFixture())).toBe('localhost:1433/Sales')
    expect(connectionSubtitle(connectionFixture({ database: null }))).toBe('localhost:1433')
    expect(connectionSubtitle(connectionFixture({ port: null, database: null }))).toBe('localhost')
    expect(connectionSubtitle(connectionFixture({ host: null, port: null, database: null }))).toBe(
      'localhost',
    )

    const sqlite = connectionFixture({ dbType: DbType.Sqlite })
    expect(connectionSubtitle(sqlite)).toBe('No file')
    sqlite.options.filePath = '/tmp/a.db'
    expect(connectionSubtitle(sqlite)).toBe('/tmp/a.db')

    const athena = connectionFixture({ dbType: DbType.Athena, database: 'logs' })
    expect(connectionSubtitle(athena)).toBe('logs')
    athena.options.awsRegion = 'us-east-1'
    expect(connectionSubtitle(athena)).toBe('us-east-1 · logs')
    expect(connectionSubtitle(connectionFixture({ dbType: DbType.Athena, database: null }))).toBe(
      'AWS',
    )
  })
})

describe('connections store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    Object.values(apiStub).forEach((fn) => fn.mockReset())
    apiStub.getConnections.mockResolvedValue([])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.supportedEngines.mockResolvedValue([])
  })

  it('names a connection, and reports a record that is gone', async () => {
    apiStub.getConnections.mockResolvedValue([{ ...connectionFixture(), name: 'Server' }])
    const connections = useConnectionsStore()
    await connections.load()
    expect(connections.nameFor('c1')).toBe('Server')
    expect(connections.nameFor('gone')).toBe('Connection that is gone')
  })

  it('reads the engines the build supports', async () => {
    apiStub.supportedEngines.mockResolvedValue([{ dbType: DbType.Mssql, label: 'MS SQL Server' }])
    const connections = useConnectionsStore()
    await connections.loadEngines()
    expect(connections.engines).toHaveLength(1)
  })

  it('reports a failure to read the engines', async () => {
    apiStub.supportedEngines.mockRejectedValue({ kind: 'internal', message: 'no', detail: null })
    const connections = useConnectionsStore()
    await connections.loadEngines()
    expect(useUiStore().notices).toHaveLength(1)
  })

  it('reads the saved and the open connections', async () => {
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    const connections = useConnectionsStore()
    await connections.load()
    expect(connections.saved).toHaveLength(1)
    expect(connections.hasActive).toBe(true)
    expect(connections.health.c1).toBe(ConnectionHealth.Connected)
    expect(connections.byId('c1')?.name).toBe('Server')
    expect(connections.byId('none')).toBeUndefined()
    expect(connections.isActive('c1')).toBe(true)
    expect(connections.loading).toBe(false)
  })

  it('reports a failure to read the connections', async () => {
    apiStub.getConnections.mockRejectedValue({ kind: 'storage', message: 'no', detail: null })
    const connections = useConnectionsStore()
    await connections.load()
    expect(useUiStore().notices[0]?.level).toBe('error')
  })

  it('groups the connections by their folder', async () => {
    apiStub.getConnections.mockResolvedValue([
      connectionFixture({ id: 'a', group: 'Production' }),
      connectionFixture({ id: 'b', group: null }),
      connectionFixture({ id: 'c', group: '  ' }),
    ])
    const connections = useConnectionsStore()
    await connections.load()
    expect(connections.groups).toEqual(['Connections', 'Production'])
  })

  it('refuses to save a record that is not complete', async () => {
    const connections = useConnectionsStore()
    const saved = await connections.save(connectionFixture({ name: '' }))
    expect(saved).toBe(false)
    expect(apiStub.saveConnection).not.toHaveBeenCalled()
    expect(useUiStore().notices[0]?.level).toBe('warning')
  })

  it('saves a complete record and reads the list again', async () => {
    apiStub.saveConnection.mockResolvedValue(undefined)
    const connections = useConnectionsStore()
    const saved = await connections.save(connectionFixture())
    expect(saved).toBe(true)
    expect(apiStub.getConnections).toHaveBeenCalled()
    expect(useUiStore().notices[0]?.level).toBe('success')
  })

  it('reports a failure to save', async () => {
    apiStub.saveConnection.mockRejectedValue({ kind: 'storage', message: 'no', detail: null })
    const connections = useConnectionsStore()
    expect(await connections.save(connectionFixture())).toBe(false)
  })

  it('removes a record and forgets its state', async () => {
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')

    apiStub.getConnections.mockResolvedValue([])
    apiStub.listActiveConnections.mockResolvedValue([])
    apiStub.deleteConnection.mockResolvedValue(undefined)
    await connections.remove('c1')
    expect(connections.selectedId).toBeNull()
    expect(connections.isActive('c1')).toBe(false)
  })

  it('reports a failure to remove a record', async () => {
    apiStub.deleteConnection.mockRejectedValue({ kind: 'storage', message: 'no', detail: null })
    const connections = useConnectionsStore()
    await connections.remove('c1')
    expect(useUiStore().notices[0]?.level).toBe('error')
  })

  it('opens a connection and selects it', async () => {
    apiStub.connect.mockResolvedValue(infoFixture())
    const connections = useConnectionsStore()
    const opened = await connections.connect(connectionFixture())
    expect(opened).toBe(true)
    expect(connections.selectedId).toBe('c1')
    expect(connections.health.c1).toBe(ConnectionHealth.Connected)
    expect(connections.connecting.c1).toBeUndefined()
  })

  it('reports a failure to open a connection', async () => {
    apiStub.connect.mockRejectedValue({ kind: 'connection', message: 'refused', detail: null })
    const connections = useConnectionsStore()
    expect(await connections.connect(connectionFixture())).toBe(false)
    expect(connections.isActive('c1')).toBe(false)
  })

  it('closes a connection and moves the selection to another open one', async () => {
    apiStub.getConnections.mockResolvedValue([
      connectionFixture({ id: 'c1' }),
      connectionFixture({ id: 'c2' }),
    ])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture('c1'), infoFixture('c2')])
    apiStub.disconnect.mockResolvedValue(undefined)
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')

    await connections.disconnect('c1')
    expect(connections.selectedId).toBe('c2')
    expect(connections.health.c1).toBe(ConnectionHealth.Disconnected)
  })

  it('clears the selection when the last connection closes', async () => {
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.disconnect.mockResolvedValue(undefined)
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')
    await connections.disconnect('c1')
    expect(connections.selectedId).toBeNull()
  })

  it('closes a connection even when the backend refuses', async () => {
    apiStub.disconnect.mockRejectedValue({ kind: 'internal', message: 'no', detail: null })
    const connections = useConnectionsStore()
    await connections.disconnect('c1')
    expect(connections.isActive('c1')).toBe(false)
  })

  it('tests a record and reports the answer', async () => {
    apiStub.testConnection.mockResolvedValue('The connection works.')
    const connections = useConnectionsStore()
    expect(await connections.test(connectionFixture())).toBe(true)
    expect(useUiStore().notices[0]?.message).toBe('The connection works.')
    expect(connections.testing).toBe(false)
  })

  it('refuses to test a record that is not complete', async () => {
    const connections = useConnectionsStore()
    expect(await connections.test(connectionFixture({ name: '' }))).toBe(false)
    expect(apiStub.testConnection).not.toHaveBeenCalled()
  })

  it('reports a failed test', async () => {
    apiStub.testConnection.mockRejectedValue({ kind: 'connection', message: 'no', detail: null })
    const connections = useConnectionsStore()
    expect(await connections.test(connectionFixture())).toBe(false)
  })

  it('records the state the backend reports', async () => {
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')

    connections.applyStatus({
      connectionId: 'c1',
      health: ConnectionHealth.Reconnecting,
      message: null,
    })
    expect(connections.health.c1).toBe(ConnectionHealth.Reconnecting)
    expect(connections.isActive('c1')).toBe(true)

    connections.applyStatus({
      connectionId: 'c1',
      health: ConnectionHealth.Disconnected,
      message: 'the socket closed',
    })
    expect(connections.isActive('c1')).toBe(false)
    expect(connections.selectedId).toBeNull()
    expect(useUiStore().notices[0]?.message).toBe('the socket closed')
  })

  it('raises no note when a closed connection reports no reason', () => {
    const connections = useConnectionsStore()
    connections.applyStatus({
      connectionId: 'c1',
      health: ConnectionHealth.Disconnected,
      message: null,
    })
    expect(useUiStore().notices).toHaveLength(0)
  })

  it('builds a copy of a record under a new name', () => {
    const connections = useConnectionsStore()
    const copy = connections.duplicate(connectionFixture())
    expect(copy.id).not.toBe('c1')
    expect(copy.name).toBe('Server (copy)')
    expect(copy.password).toBe('')
  })

  it('keeps the selection when another record is removed', async () => {
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.deleteConnection.mockResolvedValue(undefined)
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')
    await connections.remove('other')
    expect(connections.selectedId).toBe('c1')
  })

  it('leaves the selection alone when another connection closes', async () => {
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    apiStub.disconnect.mockResolvedValue(undefined)
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')
    await connections.disconnect('other')
    expect(connections.selectedId).toBe('c1')
  })

  it('leaves the selection alone when another connection reports a close', async () => {
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    const connections = useConnectionsStore()
    await connections.load()
    connections.select('c1')
    connections.applyStatus({
      connectionId: 'other',
      health: ConnectionHealth.Disconnected,
      message: null,
    })
    expect(connections.selectedId).toBe('c1')
  })

  it('finds the record and the details of the selected connection', async () => {
    apiStub.getConnections.mockResolvedValue([connectionFixture()])
    apiStub.listActiveConnections.mockResolvedValue([infoFixture()])
    const connections = useConnectionsStore()
    await connections.load()
    expect(connections.selected).toBeNull()
    expect(connections.selectedInfo).toBeUndefined()
    connections.select('ghost')
    expect(connections.selected).toBeNull()
    connections.select('c1')
    expect(connections.selected?.id).toBe('c1')
    expect(connections.selectedInfo?.connectionId).toBe('c1')
    expect(connections.activeList).toHaveLength(1)
  })
})
