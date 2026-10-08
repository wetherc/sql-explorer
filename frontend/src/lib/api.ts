import { Channel, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { ResultStream, type ResultStreamHandlers } from '@/lib/results'
import type { Dialect } from '@/types/api'
import type {
  ColumnRef,
  ConnectionInfo,
  ConstraintRef,
  ConnectionStatusEvent,
  DatabaseRef,
  EngineInfo,
  EventRef,
  ChosenMessagesFile,
  ChosenRunFile,
  ExecOptions,
  ExportRequest,
  ExportSummary,
  KeptExportRequest,
  RunFileSummary,
  FolderEntry,
  MenuCommandState,
  ObjectType,
  OpenedFile,
  TextFile,
  SaveFileRequest,
  SaveShownMessagesRequest,
  SaveStatementRequest,
  SavedStatement,
  HistoryEntry,
  IndexRef,
  PartitionList,
  PlanMode,
  QueryResponse,
  RoutineRef,
  RunToFileRequest,
  SavedConnection,
  SchemaRef,
  SchemaSnapshot,
  SpillRequest,
  ScriptStatement,
  TableDetails,
  RelationType,
  TableRef,
  TriggerRef,
} from '@/types/api'

/** The name of the event that reports a change of connection state. */
export const CONNECTION_STATUS_EVENT = 'connection-status'

/** The name of the event that carries a command of the menu of the system. */
export const MENU_COMMAND_EVENT = 'menu-command'

/** The time a run waits for its next frame after the backend answers. */
export const LAST_FRAME_WAIT_MS = 10_000

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

/**
 * Calls a command that sends the rows of a run on a channel as binary
 * chunks, and gives back the answer of the command. The handlers receive
 * each result set as it ends, and then the numbers of the run.
 */
async function streamed<T>(
  command: string,
  request: Record<string, unknown>,
  handlers: ResultStreamHandlers,
): Promise<T> {
  const stream = new ResultStream(handlers)
  const onChunk = new Channel<ArrayBuffer>()
  onChunk.onmessage = (message) => stream.feed(message)
  let failed = false
  let error: unknown = null
  let answer: T | undefined
  try {
    answer = await invoke<T>(command, { request: withNulls(request), onChunk })
  } catch (caught) {
    failed = true
    error = caught
  }
  // The backend sends the end frame for a failed run too, with the
  // messages of the server, so the wait is the same on both paths.
  await stream.settle(LAST_FRAME_WAIT_MS)
  stream.close()
  if (failed) {
    throw error
  }
  // A fault of the frames cannot travel out of the channel, so the reader
  // keeps it and the run fails here.
  const failure = stream.failure
  if (failure) {
    throw failure
  }
  return answer as T
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
  /** Opens the saved connection with this identifier. The backend reads
   *  the server and the credentials out of its own record. */
  connect(connectionId: string): Promise<ConnectionInfo> {
    return invoke('connect', { connectionId })
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

  /**
   * Runs a script. The rows arrive on a channel as binary chunks while the
   * read runs, so neither side holds the whole answer. The handlers receive
   * each result set as it ends, and then the numbers of the run.
   */
  executeQuery(
    request: {
      connectionId: string
      requestId: string
      query: string
      tabId?: string
      queryParams?: Record<string, unknown>
      options?: ExecOptions
      spill?: SpillRequest
      /** The seconds the read may pause at the row limit, or 0. */
      pauseSecs?: number
      /** The identifier of the file that gets every message of the run. */
      messagesFile?: string
    },
    handlers: ResultStreamHandlers,
  ): Promise<void> {
    return streamed<void>('execute_query', request, handlers)
  },

  /** Asks the user for the file of a run to a file. The dialog offers CSV,
   *  JSON and Excel files, and the extension sets the format. Gives back
   *  null when the user closed the dialog. */
  chooseRunFile(request: { defaultName: string }): Promise<ChosenRunFile | null> {
    return call('choose_run_file', request)
  },

  /** True when a statement can give more than one result set. The check
   *  reads the text alone, so a procedure can still give more sets. */
  severalResultSets(query: string, dialect: Dialect): Promise<boolean> {
    return invoke('several_result_sets', { query, dialect })
  },

  /**
   * Runs a script one time. The rows of the first result set, or of each
   * set, go to the file of the ticket, and the first rows of each set reach
   * the handlers as in `executeQuery`. Gives back what the files received.
   */
  runToFile(request: RunToFileRequest, handlers: ResultStreamHandlers): Promise<RunFileSummary> {
    return streamed<RunFileSummary>('run_to_file', { ...request }, handlers)
  },

  /** Asks the user for a text file that gets every message of the runs of
   *  a tab. Gives back null when the user closed the dialog. */
  chooseMessagesFile(defaultName: string): Promise<ChosenMessagesFile | null> {
    return invoke('choose_messages_file', { defaultName })
  },

  /** Stops the writes to a file of messages. */
  forgetMessagesFile(id: string): Promise<void> {
    return invoke('forget_messages_file', { id })
  },

  /** Sends the messages of a run that goes on to a chosen file. Gives back
   *  false when the run already ended. */
  saveRunMessages(requestId: string, fileId: string): Promise<boolean> {
    return invoke('save_run_messages', { requestId, fileId })
  },

  /** Asks the user for a path and writes the messages that a tab shows.
   *  Gives back the path, or null when the user closed the dialog. */
  saveShownMessages(request: SaveShownMessagesRequest): Promise<string | null> {
    return call('save_shown_messages', { ...request })
  },

  explainQuery(request: {
    connectionId: string
    requestId: string
    query: string
    mode: PlanMode
    tabId?: string
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

  /** Releases the session of one tab, when the tab closes or moves to
   *  another connection. */
  releaseSession(connectionId: string, tabId: string): Promise<void> {
    return invoke('release_session', { connectionId, tabId })
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

  listTriggers(
    connectionId: string,
    database: string,
    schemaName: string | null,
    tableName: string,
  ): Promise<TriggerRef[]> {
    return call('list_triggers', { connectionId, database, schemaName, tableName })
  },

  listEvents(
    connectionId: string,
    database: string,
    schemaName: string | null,
  ): Promise<EventRef[]> {
    return call('list_events', { connectionId, database, schemaName })
  },

  listPartitions(
    connectionId: string,
    database: string,
    schemaName: string | null,
    tableName: string,
  ): Promise<PartitionList> {
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
   * Asks the backend for one statement of an object of the tree. The statements
   * are `create`, `select`, `insert` and `update`.
   */
  scriptObject(request: {
    connectionId: string
    database: string | null
    schemaName: string | null
    /** The name of the object, which for a trigger is the name of the trigger. */
    tableName: string
    /** The relation of a trigger. */
    parentName?: string | null
    target: RelationType | ObjectType
    statement: ScriptStatement
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

  addHistoryEntry(entry: HistoryEntry): Promise<void> {
    return invoke('add_history_entry', { entry })
  },

  clearHistory(): Promise<void> {
    return invoke('clear_history')
  },

  getWorkspace(): Promise<unknown> {
    return invoke('get_workspace')
  },

  saveWorkspace(workspace: unknown): Promise<void> {
    return invoke('save_workspace', { workspace })
  },

  /** Asks the user for a folder and records it. Gives back the path, or
   *  null when the user closed the dialog. */
  pickFolder(): Promise<string | null> {
    return invoke('pick_folder')
  },

  /** The folders that the user accepted, which the backend records. A
   *  folder that is gone from the disk is not in the list. */
  fileRoots(): Promise<string[]> {
    return invoke('file_roots')
  },

  /** Takes one folder out of the record, so no path under it is reachable
   *  any more. */
  closeFolder(path: string): Promise<void> {
    return invoke('close_folder', { path })
  },

  /** Asks the user for one statement file and reads it. The folder of that
   *  file becomes a root. Gives back null when the user closed the dialog. */
  openStatementFile(): Promise<OpenedFile | null> {
    return invoke('open_statement_file')
  },

  /** Lists the entries of one folder that the user opened. */
  listFolder(path: string): Promise<FolderEntry[]> {
    return invoke('list_folder', { path })
  },

  /** Reads the text of one file inside a folder that the user opened, with
   *  the encoding that the backend found. */
  readTextFile(path: string): Promise<TextFile> {
    return invoke('read_text_file', { path })
  },

  /** Writes the statement of a tab. A request with a path that the user
   *  accepted, through a dialog or an open folder, writes that file at once.
   *  Any other request opens the save dialog, which starts at the file of the
   *  request when it has one, and the chosen file becomes accepted. The file
   *  is in UTF-8 when the request gives no encoding. Text that Windows-1252
   *  can't store is written as UTF-8 with a mark. Gives back the path and the
   *  encoding of the file, or null when the user closed the dialog. */
  saveStatementFile(request: SaveStatementRequest): Promise<SavedStatement | null> {
    return invoke('save_statement_file', { request })
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

  /** Writes every row of a kept result to a file the user chooses, without
   *  a new run of the query. Gives back null when the user closed the
   *  dialog. */
  exportKept(request: KeptExportRequest): Promise<ExportSummary | null> {
    return invoke('export_kept', { request })
  },

  /** Lets the backend forget a kept result that left the interface. */
  releaseKept(keptId: string): Promise<void> {
    return invoke('release_kept', { keptId })
  },

  /** Asks the user for a path and writes bytes there. The content travels
   *  as base64 text. Gives back null when the user closed the dialog. */
  saveBinaryFile(request: SaveFileRequest): Promise<string | null> {
    return invoke('save_binary_file', { request })
  },

  supportedEngines(): Promise<EngineInfo[]> {
    return invoke('supported_engines')
  },

  /** False when the keychain of the system was not reachable, so a saved
   *  password stays for this session only. */
  passwordsPersist(): Promise<boolean> {
    return invoke('passwords_persist')
  },

  /** The problems the backend met while it read the saved files, such as a
   *  file it could not read and set aside. */
  storageProblems(): Promise<string[]> {
    return invoke('storage_problems')
  },

  onConnectionStatus(handler: (event: ConnectionStatusEvent) => void): Promise<UnlistenFn> {
    return listen<ConnectionStatusEvent>(CONNECTION_STATUS_EVENT, (event) => handler(event.payload))
  },

  /** Tells the backend which commands of the menu can run now, so the
   *  operating system greys out the ones that cannot. */
  setMenuCommands(states: MenuCommandState[]): Promise<void> {
    return invoke('set_menu_commands', { states })
  },

  /** Hears the menu of the operating system. The payload is the identifier
   *  of the command that the user chose. */
  onMenuCommand(handler: (id: string) => void): Promise<UnlistenFn> {
    return listen<string>(MENU_COMMAND_EVENT, (event) => handler(event.payload))
  },
}

export type Api = typeof api
