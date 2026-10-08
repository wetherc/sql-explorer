import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
const listen = vi.fn()

class ChannelStub {
  onmessage: ((message: ArrayBuffer) => void) | null = null
}

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  Channel: ChannelStub,
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: (...args: unknown[]) => listen(...args) }))

const { api, CONNECTION_STATUS_EVENT, LAST_FRAME_WAIT_MS } = await import('@/lib/api')

/** The handlers of a run, which these tests do not need to answer. */
function handlers() {
  return { onSet: () => {}, onEnd: vi.fn() }
}

/** The frame that ends a run, as the backend writes it. */
function endFrame(messages: unknown[] = []): ArrayBuffer {
  const json = new TextEncoder().encode(JSON.stringify({ messages, elapsedMs: 1 }))
  const bytes = new Uint8Array(5 + json.length)
  bytes[0] = 4
  new DataView(bytes.buffer).setUint32(1, json.length, true)
  bytes.set(json, 5)
  return bytes.buffer
}

/** Answers every command, and ends the channel of a run before it answers. */
function answer(_command: string, args?: { onChunk?: ChannelStub }): Promise<unknown> {
  args?.onChunk?.onmessage?.(endFrame())
  return Promise.resolve(undefined)
}
const { newConnection } = await import('@/stores/connections')
const { Dialect } = await import('@/types/api')

describe('api', () => {
  beforeEach(() => {
    invoke.mockReset().mockImplementation(answer)
    listen.mockReset().mockResolvedValue(() => {})
  })

  it('sends the identifier alone when it opens a connection', async () => {
    await api.connect('c1')
    expect(invoke).toHaveBeenCalledWith('connect', { connectionId: 'c1' })
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

    await api.releaseSession('c1', 't1')
    expect(invoke).toHaveBeenCalledWith('release_session', { connectionId: 'c1', tabId: 't1' })

    await api.listDatabases('c1')
    expect(invoke).toHaveBeenCalledWith('list_databases', { connectionId: 'c1' })

    await api.listSchemas('c1', 'db')
    expect(invoke).toHaveBeenCalledWith('list_schemas', { connectionId: 'c1', database: 'db' })

    await api.listTables('c1', 'db', 'dbo')
    expect(invoke).toHaveBeenCalledWith('list_tables', {
      request: { connectionId: 'c1', database: 'db', schemaName: 'dbo' },
    })

    await api.listColumns('c1', 'db', 'dbo', 't')
    expect(invoke).toHaveBeenCalledWith('list_columns', {
      request: { connectionId: 'c1', database: 'db', schemaName: 'dbo', tableName: 't' },
    })

    await api.listRoutines('c1', 'db', 'dbo')
    expect(invoke).toHaveBeenCalledWith('list_routines', {
      request: { connectionId: 'c1', database: 'db', schemaName: 'dbo' },
    })

    await api.listEvents('c1', 'db', null)
    expect(invoke).toHaveBeenCalledWith('list_events', {
      request: { connectionId: 'c1', database: 'db', schemaName: null },
    })

    for (const [method, command] of [
      ['listIndexes', 'list_indexes'],
      ['listConstraints', 'list_constraints'],
      ['listPartitions', 'list_partitions'],
      ['listTriggers', 'list_triggers'],
    ] as const) {
      await api[method]('c1', 'db', 'dbo', 't')
      expect(invoke).toHaveBeenCalledWith(command, {
        request: { connectionId: 'c1', database: 'db', schemaName: 'dbo', tableName: 't' },
      })
    }

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

    await api.getWorkspace()
    expect(invoke).toHaveBeenCalledWith('get_workspace')

    await api.saveWorkspace({ tabs: [] })
    expect(invoke).toHaveBeenCalledWith('save_workspace', { workspace: { tabs: [] } })

    await api.pickFolder()
    expect(invoke).toHaveBeenCalledWith('pick_folder')

    await api.fileRoots()
    expect(invoke).toHaveBeenCalledWith('file_roots')

    await api.closeFolder('/data')
    expect(invoke).toHaveBeenCalledWith('close_folder', { path: '/data' })

    await api.listFolder('/data')
    expect(invoke).toHaveBeenCalledWith('list_folder', { path: '/data' })

    invoke.mockResolvedValueOnce({ contents: 'SELECT 2', encoding: 'windows1252' })
    expect(await api.readTextFile('/data/a.sql')).toEqual({
      contents: 'SELECT 2',
      encoding: 'windows1252',
    })
    expect(invoke).toHaveBeenCalledWith('read_text_file', { path: '/data/a.sql' })

    await api.saveTextFile({
      defaultName: 'a.csv',
      filterLabel: 'CSV',
      extension: 'csv',
      contents: 'a,b',
    })
    expect(invoke).toHaveBeenCalledWith('save_text_file', {
      request: expect.objectContaining({ defaultName: 'a.csv', contents: 'a,b' }),
    })

    await api.exportQuery({
      connectionId: 'c1',
      requestId: 'r1',
      query: 'SELECT 1',
      defaultName: 'all.csv',
      format: 'csv',
      maxRows: 1000,
    })
    expect(invoke).toHaveBeenCalledWith('export_query', {
      request: expect.objectContaining({ defaultName: 'all.csv' }),
    })

    const kept = {
      keptId: 'r1:0',
      requestId: 'e1',
      defaultName: 'all.csv',
      format: 'csv' as const,
      maxRows: 1000,
    }
    await api.exportKept(kept)
    expect(invoke).toHaveBeenCalledWith('export_kept', { request: kept })
    await api.releaseKept('r1:0')
    expect(invoke).toHaveBeenCalledWith('release_kept', { keptId: 'r1:0' })

    await api.saveBinaryFile({
      defaultName: 'a.xlsx',
      filterLabel: 'Excel',
      extension: 'xlsx',
      contents: 'UEs=',
    })
    expect(invoke).toHaveBeenCalledWith('save_binary_file', {
      request: expect.objectContaining({ defaultName: 'a.xlsx', contents: 'UEs=' }),
    })

    await api.supportedEngines()
    expect(invoke).toHaveBeenCalledWith('supported_engines')

    await api.passwordsPersist()
    expect(invoke).toHaveBeenCalledWith('passwords_persist')

    await api.storageProblems()
    expect(invoke).toHaveBeenCalledWith('storage_problems')

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

    await api.openStatementFile()
    expect(invoke).toHaveBeenCalledWith('open_statement_file')

    await api.saveStatementFile({
      defaultName: 'a.sql',
      defaultFolder: null,
      contents: 'SELECT 1',
    })
    expect(invoke).toHaveBeenCalledWith('save_statement_file', {
      request: expect.objectContaining({ defaultName: 'a.sql' }),
    })

    const states = [{ id: 'run', enabled: true }]
    await api.setMenuCommands(states)
    expect(invoke).toHaveBeenCalledWith('set_menu_commands', { states })
  })

  it('hears the menu of the operating system', async () => {
    const handler = vi.fn()
    listen.mockImplementation((_name: string, listener: (event: unknown) => void) => {
      listener({ payload: 'run' })
      return Promise.resolve(() => {})
    })

    await api.onMenuCommand(handler)
    expect(handler).toHaveBeenCalledWith('run')
  })

  it('chooses, attaches and forgets the file of the messages of a tab', async () => {
    await api.chooseMessagesFile('messages.txt')
    expect(invoke).toHaveBeenLastCalledWith('choose_messages_file', {
      defaultName: 'messages.txt',
    })
    await api.saveRunMessages('r1', 'f1')
    expect(invoke).toHaveBeenLastCalledWith('save_run_messages', {
      requestId: 'r1',
      fileId: 'f1',
    })
    await api.forgetMessagesFile('f1')
    expect(invoke).toHaveBeenLastCalledWith('forget_messages_file', { id: 'f1' })
    const messages = [{ level: 'info' as const, text: 'a', detail: null }]
    await api.saveShownMessages({ defaultName: 'm.txt', messages, dropped: 2 })
    expect(invoke).toHaveBeenLastCalledWith('save_shown_messages', {
      request: { defaultName: 'm.txt', messages, dropped: 2 },
    })
  })

  it('asks for the file of a run and gives back the answer of the run', async () => {
    await api.chooseRunFile({ defaultName: 'q.csv' })
    expect(invoke).toHaveBeenCalledWith('choose_run_file', { request: { defaultName: 'q.csv' } })

    const summary = { rows: 2, truncated: false, path: '/a/q.csv' }
    invoke.mockImplementation((_command: string, args: { onChunk: ChannelStub }) => {
      args.onChunk.onmessage?.(endFrame())
      return Promise.resolve(summary)
    })
    const run = handlers()
    const answer = await api.runToFile(
      { connectionId: 'c1', requestId: 'r1', query: 'SELECT 1', ticket: 'k1', maxRows: 9 },
      run,
    )
    expect(answer).toBe(summary)
    expect(run.onEnd).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenLastCalledWith('run_to_file', {
      request: { connectionId: 'c1', requestId: 'r1', query: 'SELECT 1', ticket: 'k1', maxRows: 9 },
      onChunk: expect.anything(),
    })
  })

  it('sends the limits of an execution when they are given', async () => {
    await api.executeQuery(
      {
        connectionId: 'c1',
        requestId: 'r1',
        query: 'SELECT 1',
        options: { maxRows: 10, timeoutSecs: 5 },
      },
      handlers(),
    )
    expect(invoke).toHaveBeenCalledWith('execute_query', {
      request: {
        connectionId: 'c1',
        requestId: 'r1',
        query: 'SELECT 1',
        options: { maxRows: 10, timeoutSecs: 5 },
      },
      // The channel carries the rows of the run.
      onChunk: expect.anything(),
    })
  })

  it('reports a fault of the frames once the backend answers', async () => {
    invoke.mockImplementation((_command: string, args: { onChunk: ChannelStub }) => {
      // A frame of an unknown type reaches the reader through the channel.
      args.onChunk.onmessage?.(new Uint8Array([99]).buffer)
      return Promise.resolve(undefined)
    })

    await expect(
      api.executeQuery({ connectionId: 'c1', requestId: 'r1', query: 'SELECT 1' }, handlers()),
    ).rejects.toThrow(/frame of unknown type 99/)
  })

  it('waits for the frames that arrive after the backend answers', async () => {
    let channel: ChannelStub | null = null
    invoke.mockImplementation((_command: string, args: { onChunk: ChannelStub }) => {
      channel = args.onChunk
      // The bridge fetches a large message on its own, so the end frame
      // comes after the answer of the command.
      setTimeout(() => channel?.onmessage?.(endFrame()), 0)
      return Promise.resolve(undefined)
    })
    const run = handlers()
    await api.executeQuery({ connectionId: 'c1', requestId: 'r1', query: 'SELECT 1' }, run)
    expect(run.onEnd).toHaveBeenCalledTimes(1)

    // A frame that comes after the end of the run reaches no handler.
    channel!.onmessage?.(endFrame())
    expect(run.onEnd).toHaveBeenCalledTimes(1)
  })

  it('gives the error of a failed run after its end frame', async () => {
    invoke.mockImplementation((_command: string, args: { onChunk: ChannelStub }) => {
      setTimeout(() => args.onChunk.onmessage?.(endFrame([{ level: 'error', text: 'x' }])), 0)
      return Promise.reject(new Error('The statement failed.'))
    })
    const run = handlers()
    await expect(
      api.executeQuery({ connectionId: 'c1', requestId: 'r1', query: 'SELECT 1' }, run),
    ).rejects.toThrow('The statement failed.')
    expect(run.onEnd).toHaveBeenCalledWith(
      expect.objectContaining({ messages: [{ level: 'error', text: 'x' }] }),
    )
  })

  it('fails a run whose end frame does not arrive', async () => {
    vi.useFakeTimers()
    try {
      invoke.mockResolvedValue(undefined)
      const run = api.executeQuery(
        { connectionId: 'c1', requestId: 'r1', query: 'SELECT 1' },
        handlers(),
      )
      const outcome = expect(run).rejects.toThrow('The last rows of the result never arrived.')
      await vi.advanceTimersByTimeAsync(LAST_FRAME_WAIT_MS)
      await outcome
    } finally {
      vi.useRealTimers()
    }
  })

  it('leaves the limits out when none are given', async () => {
    await api.executeQuery({ connectionId: 'c1', requestId: 'r1', query: 'SELECT 1' }, handlers())
    expect(invoke).toHaveBeenCalledWith('execute_query', {
      request: { connectionId: 'c1', requestId: 'r1', query: 'SELECT 1' },
      onChunk: expect.anything(),
    })
  })

  it('sends null for a field the caller set to undefined', async () => {
    await api.executeQuery(
      {
        connectionId: 'c1',
        requestId: 'r1',
        query: 'SELECT 1',
        queryParams: undefined,
        options: undefined,
      },
      handlers(),
    )
    expect(invoke).toHaveBeenCalledWith('execute_query', {
      request: {
        connectionId: 'c1',
        requestId: 'r1',
        query: 'SELECT 1',
        queryParams: null,
        options: null,
      },
      onChunk: expect.anything(),
    })
  })

  it('asks for the plan of a statement, with and without limits', async () => {
    await api.explainQuery({
      connectionId: 'c1',
      requestId: 'r1',
      query: 'SELECT 1',
      mode: 'actual',
      options: { maxRows: 10, timeoutSecs: 5 },
    })
    expect(invoke).toHaveBeenCalledWith('explain_query', {
      request: {
        connectionId: 'c1',
        requestId: 'r1',
        query: 'SELECT 1',
        mode: 'actual',
        options: { maxRows: 10, timeoutSecs: 5 },
      },
    })

    await api.explainQuery({
      connectionId: 'c1',
      requestId: 'r1',
      query: 'SELECT 1',
      mode: 'estimated',
    })
    expect(invoke).toHaveBeenCalledWith('explain_query', {
      request: { connectionId: 'c1', requestId: 'r1', query: 'SELECT 1', mode: 'estimated' },
    })

    await api.queryParameters('SELECT :id', Dialect.MsSql)
    expect(invoke).toHaveBeenCalledWith('query_parameters', {
      query: 'SELECT :id',
      dialect: Dialect.MsSql,
    })

    await api.severalResultSets('SELECT 1; SELECT 2', Dialect.MsSql)
    expect(invoke).toHaveBeenCalledWith('several_result_sets', {
      query: 'SELECT 1; SELECT 2',
      dialect: Dialect.MsSql,
    })

    await api.runFileReady('k1')
    expect(invoke).toHaveBeenCalledWith('run_file_ready', { ticket: 'k1' })
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
      request: {
        connectionId: 'c1',
        database: 'db',
        schemaName: 'dbo',
        tableName: 't',
        limit: 50,
      },
    })

    await api.previewQuery({
      connectionId: 'c1',
      database: null,
      schemaName: null,
      tableName: 't',
    })
    expect(invoke).toHaveBeenCalledWith('preview_query', {
      request: { connectionId: 'c1', database: null, schemaName: null, tableName: 't' },
    })
  })

  it('names the relation whose parts it reads', async () => {
    await api.tableDetails('c1', 'db', 'dbo', 't')
    expect(invoke).toHaveBeenCalledWith('table_details', {
      request: { connectionId: 'c1', database: 'db', schemaName: 'dbo', tableName: 't' },
    })
  })

  it('sends null for a schema that the caller left empty', async () => {
    await api.listTables('c1', 'db', null)
    expect(invoke).toHaveBeenCalledWith('list_tables', {
      request: { connectionId: 'c1', database: 'db', schemaName: null },
    })

    await api.listColumns('c1', 'db', undefined as unknown as string | null, 't')
    expect(invoke).toHaveBeenCalledWith('list_columns', {
      request: { connectionId: 'c1', database: 'db', schemaName: null, tableName: 't' },
    })
  })

  it('sends the bounds of a read of a schema', async () => {
    await api.schemaSnapshot({
      connectionId: 'c1',
      database: 'db',
      maxColumns: 100,
      ownConnection: false,
    })
    expect(invoke).toHaveBeenCalledWith('schema_snapshot', {
      request: {
        connectionId: 'c1',
        database: 'db',
        maxColumns: 100,
        ownConnection: false,
      },
    })
  })

  it('sends the request of a statement as one record', async () => {
    const request = {
      connectionId: 'c1',
      database: 'db',
      schemaName: 'dbo',
      tableName: 't',
      target: 'table' as const,
      statement: 'insert' as const,
    }
    await api.scriptObject(request)
    expect(invoke).toHaveBeenCalledWith('script_object', { request })
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
