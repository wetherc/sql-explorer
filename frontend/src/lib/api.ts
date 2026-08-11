import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { Dialect } from '@/types/api'
import type {
  ColumnRef,
  ConnectionInfo,
  ConstraintRef,
  ConnectionStatusEvent,
  DatabaseRef,
  EngineInfo,
  ExecOptions,
  ExportRequest,
  ExportSummary,
  SaveFileRequest,
  HistoryEntry,
  IndexRef,
  PartitionRef,
  PlanKind,
  QueryResponse,
  RoutineRef,
  SavedConnection,
  SavedQuery,
  SchemaRef,
  SchemaSnapshot,
  ScriptKind,
  TableDetails,
  TableRef,
} from '@/types/api'

/** The name of the event that reports a change of connection state. */
export const CONNECTION_STATUS_EVENT = 'connection-status'

/**
 * The convention for the shape of a command: a command with more than two
 * fields takes one `request` record, and a command with one or two plain
 * fields takes them flat. A new command follows this convention.
 *
 * The backend reads the record with `serde(default)` on the optional
 * fields, so an absent field is safe. This helper also turns `undefined`
 * into `null`, so a caller can pass either and the wire carries one form.
 */
function call<T>(command: string, request: Record<string, unknown>): Promise<T> {
  return invoke(command, { request: withNulls(request) })
}

/** Replaces `undefined` with `null` in the fields of a record. */
function withNulls(value: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const [key, entry] of Object.entries(value)) {
    out[key] = entry === undefined ? null : entry
  }
  return out
}

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
    queryParams?: Record<string, unknown>
    options?: ExecOptions
  }): Promise<QueryResponse> {
    return call('execute_query', request)
  },

  explainQuery(request: {
    connectionId: string
    requestId: string
    query: string
    kind: PlanKind
    queryParams?: Record<string, unknown>
    options?: ExecOptions
  }): Promise<QueryResponse> {
    return call('explain_query', request)
  },

  /** Lists the names of the parameters that a statement holds. */
  queryParameters(query: string, dialect: Dialect): Promise<string[]> {
    return invoke('query_parameters', { query, dialect })
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
    return call('list_tables', { connectionId, database, schemaName })
  },

  listColumns(
    connectionId: string,
    database: string,
    schemaName: string | null,
    tableName: string,
  ): Promise<ColumnRef[]> {
    return call('list_columns', { connectionId, database, schemaName, tableName })
  },

  listRoutines(
    connectionId: string,
    database: string,
    schemaName: string | null,
  ): Promise<RoutineRef[]> {
    return call('list_routines', { connectionId, database, schemaName })
  },

  listIndexes(
    connectionId: string,
    database: string,
    schemaName: string | null,
    tableName: string,
  ): Promise<IndexRef[]> {
    return call('list_indexes', { connectionId, database, schemaName, tableName })
  },

  listConstraints(
    connectionId: string,
    database: string,
    schemaName: string | null,
    tableName: string,
  ): Promise<ConstraintRef[]> {
    return call('list_constraints', { connectionId, database, schemaName, tableName })
  },

  listPartitions(
    connectionId: string,
    database: string,
    schemaName: string | null,
    tableName: string,
  ): Promise<PartitionRef[]> {
    return call('list_partitions', { connectionId, database, schemaName, tableName })
  },

  /**
   * Reads the facts, the columns, the indexes and the constraints of one
   * relation, for the properties dialog.
   */
  tableDetails(
    connectionId: string,
    database: string,
    schemaName: string | null,
    tableName: string,
  ): Promise<TableDetails> {
    return call('table_details', { connectionId, database, schemaName, tableName })
  },

  /**
   * Reads every relation and every column of one database, for the
   * completions of the editor.
   */
  schemaSnapshot(request: {
    connectionId: string
    database: string
    maxColumns: number
    ownConnection: boolean
  }): Promise<SchemaSnapshot> {
    return call('schema_snapshot', request)
  },

  /**
   * Asks the backend for one statement of an object of the tree. The kinds
   * are `create`, `select`, `insert` and `update`.
   */
  scriptObject(request: {
    connectionId: string
    database: string | null
    schemaName: string | null
    tableName: string
    kind: 'table' | 'view'
    scriptKind: ScriptKind
  }): Promise<string> {
    return call('script_object', request)
  },

  previewQuery(request: {
    connectionId: string
    database: string | null
    schemaName: string | null
    tableName: string
    limit?: number
  }): Promise<string> {
    return call('preview_query', request)
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

  /** Asks the user for a path and writes text there. Gives back the path,
   *  or null when the user closed the dialog. */
  saveTextFile(request: SaveFileRequest): Promise<string | null> {
    return invoke('save_text_file', { request })
  },

  /** Runs the statement again and writes the rows to a file the user
   *  chooses. Gives back null when the user closed the dialog. */
  exportQuery(request: ExportRequest): Promise<ExportSummary | null> {
    return invoke('export_query', { request })
  },

  /** Asks the user for a path and writes bytes there. The content travels
   *  as base64 text. Gives back null when the user closed the dialog. */
  saveBinaryFile(request: SaveFileRequest): Promise<string | null> {
    return invoke('save_binary_file', { request })
  },

  supportedEngines(): Promise<EngineInfo[]> {
    return invoke('supported_engines')
  },

  onConnectionStatus(handler: (event: ConnectionStatusEvent) => void): Promise<UnlistenFn> {
    return listen<ConnectionStatusEvent>(CONNECTION_STATUS_EVENT, (event) => handler(event.payload))
  },
}

export type Api = typeof api
