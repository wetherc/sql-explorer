import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
const listen = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: (...args: unknown[]) => listen(...args) }))

const { api, CONNECTION_STATUS_EVENT } = await import('@/lib/api')
const { newConnection } = await import('@/stores/connections')

describe('api', () => {
  beforeEach(() => {
    invoke.mockReset().mockResolvedValue(undefined)
    listen.mockReset().mockResolvedValue(() => {})
  })

  it('sends the whole record when it opens a connection', async () => {
    const connection = newConnection()
    await api.connect(connection)
    expect(invoke).toHaveBeenCalledWith('connect', { connection })
  })

  it('names each command with the arguments the backend expects', async () => {
    const connection = newConnection()
    await api.testConnection(connection)
    expect(invoke).toHaveBeenCalledWith('test_connection', { connection })

    await api.disconnect('c1')
    expect(invoke).toHaveBeenCalledWith('disconnect', { connectionId: 'c1' })

    await api.listActiveConnections()
    expect(invoke).toHaveBeenCalledWith('list_active_connections')

    await api.cancelQuery('c1', 'r1')
    expect(invoke).toHaveBeenCalledWith('cancel_query', { connectionId: 'c1', requestId: 'r1' })

    await api.listDatabases('c1')
    expect(invoke).toHaveBeenCalledWith('list_databases', { connectionId: 'c1' })

    await api.listSchemas('c1', 'db')
    expect(invoke).toHaveBeenCalledWith('list_schemas', { connectionId: 'c1', database: 'db' })

    await api.listTables('c1', 'db', 'dbo')
    expect(invoke).toHaveBeenCalledWith('list_tables', {
      connectionId: 'c1',
      database: 'db',
      schemaName: 'dbo',
    })

    await api.listColumns('c1', 'db', 'dbo', 't')
    expect(invoke).toHaveBeenCalledWith('list_columns', {
      connectionId: 'c1',
      database: 'db',
      schemaName: 'dbo',
      tableName: 't',
    })

    await api.quoteIdentifier('c1', 'a b')
    expect(invoke).toHaveBeenCalledWith('quote_identifier', { connectionId: 'c1', name: 'a b' })

    await api.getConnections()
    expect(invoke).toHaveBeenCalledWith('get_connections')

    await api.saveConnection(connection)
    expect(invoke).toHaveBeenCalledWith('save_connection', { connection })

    await api.deleteConnection('c1')
    expect(invoke).toHaveBeenCalledWith('delete_connection', { id: 'c1' })

    await api.getHistory()
    expect(invoke).toHaveBeenCalledWith('get_history')

    await api.clearHistory()
    expect(invoke).toHaveBeenCalledWith('clear_history')

    await api.getSavedQueries()
    expect(invoke).toHaveBeenCalledWith('get_saved_queries')

    await api.deleteSavedQuery('q1')
    expect(invoke).toHaveBeenCalledWith('delete_saved_query', { id: 'q1' })

    await api.getWorkspace()
    expect(invoke).toHaveBeenCalledWith('get_workspace')

    await api.saveWorkspace({ tabs: [] })
    expect(invoke).toHaveBeenCalledWith('save_workspace', { workspace: { tabs: [] } })

    await api.writeTextFile('/tmp/a.csv', 'a,b')
    expect(invoke).toHaveBeenCalledWith('write_text_file', {
      path: '/tmp/a.csv',
      contents: 'a,b',
    })

    await api.readTextFile('/tmp/a.csv')
    expect(invoke).toHaveBeenCalledWith('read_text_file', { path: '/tmp/a.csv' })

    await api.supportedEngines()
    expect(invoke).toHaveBeenCalledWith('supported_engines')

    const entry = {
      id: 'h1',
      connectionId: 'c1',
      connectionName: 'n',
      query: 'SELECT 1',
      ranAt: 'now',
      elapsedMs: 1,
      rowCount: 1,
      succeeded: true,
    }
    await api.addHistoryEntry(entry)
    expect(invoke).toHaveBeenCalledWith('add_history_entry', { entry })

    const query = { id: 'q1', name: 'n', query: 'SELECT 1', updatedAt: 'now' }
    await api.saveQuery(query)
    expect(invoke).toHaveBeenCalledWith('save_query', { query })
  })

  it('sends the limits of an execution when they are given', async () => {
    await api.executeQuery({
      connectionId: 'c1',
      requestId: 'r1',
      query: 'SELECT 1',
      options: { maxRows: 10, timeoutSecs: 5 },
    })
    expect(invoke).toHaveBeenCalledWith('execute_query', {
      connectionId: 'c1',
      requestId: 'r1',
      query: 'SELECT 1',
      queryParams: null,
      options: { maxRows: 10, timeoutSecs: 5 },
    })
  })

  it('leaves the limits out when none are given', async () => {
    await api.executeQuery({ connectionId: 'c1', requestId: 'r1', query: 'SELECT 1' })
    expect(invoke).toHaveBeenCalledWith('execute_query', expect.objectContaining({ options: null }))
  })

  it('sends the preview request with and without a limit', async () => {
    await api.previewQuery({
      connectionId: 'c1',
      database: 'db',
      schemaName: 'dbo',
      tableName: 't',
      limit: 50,
    })
    expect(invoke).toHaveBeenCalledWith('preview_query', {
      connectionId: 'c1',
      database: 'db',
      schemaName: 'dbo',
      tableName: 't',
      limit: 50,
    })

    await api.previewQuery({
      connectionId: 'c1',
      database: null,
      schemaName: null,
      tableName: 't',
    })
    expect(invoke).toHaveBeenCalledWith('preview_query', expect.objectContaining({ limit: null }))
  })

  it('passes the payload of a state event to the handler', async () => {
    const handler = vi.fn()
    await api.onConnectionStatus(handler)
    expect(listen).toHaveBeenCalledWith(CONNECTION_STATUS_EVENT, expect.any(Function))

    const inner = listen.mock.calls[0]?.[1] as (event: { payload: unknown }) => void
    inner({ payload: { connectionId: 'c1', health: 'connected', message: null } })
    expect(handler).toHaveBeenCalledWith({
      connectionId: 'c1',
      health: 'connected',
      message: null,
    })
  })
})
