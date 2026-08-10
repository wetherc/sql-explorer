import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type {
  ColumnRef,
  ConnectionInfo,
  ConnectionStatusEvent,
  DatabaseRef,
  EngineInfo,
  ExecOptions,
  HistoryEntry,
  QueryResponse,
  SavedConnection,
  SavedQuery,
  SchemaRef,
  TableRef,
} from '@/types/api'

/** The name of the event that reports a change of connection state. */
export const CONNECTION_STATUS_EVENT = 'connection-status'

/**
 * Every call the interface makes to the backend. Keeping them in one place
 * means a command name appears once, and the tests replace one module.
 */
export const api = {
  connect(connection: SavedConnection): Promise<ConnectionInfo> {
    return invoke('connect', { connection })
  },

  testConnection(connection: SavedConnection): Promise<string> {
    return invoke('test_connection', { connection })
  },

  disconnect(connectionId: string): Promise<void> {
    return invoke('disconnect', { connectionId })
  },

  listActiveConnections(): Promise<ConnectionInfo[]> {
    return invoke('list_active_connections')
  },

  executeQuery(request: {
    connectionId: string
    requestId: string
    query: string
    options?: ExecOptions
  }): Promise<QueryResponse> {
    return invoke('execute_query', {
      connectionId: request.connectionId,
      requestId: request.requestId,
      query: request.query,
      queryParams: null,
      options: request.options ?? null,
    })
  },

  cancelQuery(connectionId: string, requestId: string): Promise<void> {
    return invoke('cancel_query', { connectionId, requestId })
  },

  listDatabases(connectionId: string): Promise<DatabaseRef[]> {
    return invoke('list_databases', { connectionId })
  },

  listSchemas(connectionId: string, database: string): Promise<SchemaRef[]> {
    return invoke('list_schemas', { connectionId, database })
  },

  listTables(
    connectionId: string,
    database: string,
    schemaName: string | null,
  ): Promise<TableRef[]> {
    return invoke('list_tables', { connectionId, database, schemaName })
  },

  listColumns(
    connectionId: string,
    database: string,
    schemaName: string | null,
    tableName: string,
  ): Promise<ColumnRef[]> {
    return invoke('list_columns', { connectionId, database, schemaName, tableName })
  },

  previewQuery(request: {
    connectionId: string
    database: string | null
    schemaName: string | null
    tableName: string
    limit?: number
  }): Promise<string> {
    return invoke('preview_query', {
      connectionId: request.connectionId,
      database: request.database,
      schemaName: request.schemaName,
      tableName: request.tableName,
      limit: request.limit ?? null,
    })
  },

  quoteIdentifier(connectionId: string, name: string): Promise<string> {
    return invoke('quote_identifier', { connectionId, name })
  },

  getConnections(): Promise<SavedConnection[]> {
    return invoke('get_connections')
  },

  saveConnection(connection: SavedConnection): Promise<void> {
    return invoke('save_connection', { connection })
  },

  deleteConnection(id: string): Promise<void> {
    return invoke('delete_connection', { id })
  },

  getHistory(): Promise<HistoryEntry[]> {
    return invoke('get_history')
  },

  addHistoryEntry(entry: HistoryEntry): Promise<HistoryEntry[]> {
    return invoke('add_history_entry', { entry })
  },

  clearHistory(): Promise<void> {
    return invoke('clear_history')
  },

  getSavedQueries(): Promise<SavedQuery[]> {
    return invoke('get_saved_queries')
  },

  saveQuery(query: SavedQuery): Promise<void> {
    return invoke('save_query', { query })
  },

  deleteSavedQuery(id: string): Promise<void> {
    return invoke('delete_saved_query', { id })
  },

  getWorkspace(): Promise<unknown> {
    return invoke('get_workspace')
  },

  saveWorkspace(workspace: unknown): Promise<void> {
    return invoke('save_workspace', { workspace })
  },

  writeTextFile(path: string, contents: string): Promise<void> {
    return invoke('write_text_file', { path, contents })
  },

  writeBinaryFile(path: string, contentsBase64: string): Promise<void> {
    return invoke('write_binary_file', { path, contentsBase64 })
  },

  readTextFile(path: string): Promise<string> {
    return invoke('read_text_file', { path })
  },

  supportedEngines(): Promise<EngineInfo[]> {
    return invoke('supported_engines')
  },

  onConnectionStatus(handler: (event: ConnectionStatusEvent) => void): Promise<UnlistenFn> {
    return listen<ConnectionStatusEvent>(CONNECTION_STATUS_EVENT, (event) => handler(event.payload))
  },
}

export type Api = typeof api
