// The shapes the backend sends and receives. They mirror the Rust types.

export const DbType = {
  Mssql: 'mssql',
  Athena: 'athena',
  Mysql: 'mysql',
  Postgres: 'postgres',
  Sqlite: 'sqlite',
} as const
export type DbType = (typeof DbType)[keyof typeof DbType]

export const Dialect = {
  MsSql: 'msSql',
  MySql: 'mySql',
  Postgres: 'postgres',
  Sqlite: 'sqlite',
  Athena: 'athena',
} as const
export type Dialect = (typeof Dialect)[keyof typeof Dialect]

export const TlsMode = {
  Disable: 'disable',
  Prefer: 'prefer',
  Require: 'require',
  VerifyFull: 'verifyFull',
} as const
export type TlsMode = (typeof TlsMode)[keyof typeof TlsMode]

/** How a MS SQL Server connection proves who the user is. */
export const MssqlAuth = {
  SqlLogin: 'sqlLogin',
  Integrated: 'integrated',
  EntraAzureCli: 'entraAzureCli',
  EntraAccessToken: 'entraAccessToken',
} as const
export type MssqlAuth = (typeof MssqlAuth)[keyof typeof MssqlAuth]

/** Where an Athena connection takes its AWS credentials from. */
export const AwsCredentialSource = {
  /** The default chain of the AWS tools. */
  Chain: 'chain',
  /** The keys that the user typed into the form. */
  Keys: 'keys',
} as const
export type AwsCredentialSource = (typeof AwsCredentialSource)[keyof typeof AwsCredentialSource]

export interface ConnectionOptions {
  tlsMode: TlsMode
  caCertPath: string | null
  connectTimeoutSecs: number
  queryTimeoutSecs: number
  maxRows: number
  /** The largest number of sessions the editor tabs open at one time. */
  maxSessions: number
  readOnly: boolean
  applicationName: string | null
  instanceName: string | null
  integratedSecurity: boolean
  /** How a MS SQL Server connection proves who the user is. */
  mssqlAuth: MssqlAuth
  /** The path of the Azure CLI, when the application cannot find it. */
  azureCliPath: string | null
  filePath: string | null
  awsRegion: string | null
  awsProfile: string | null
  /** Where the connection takes its AWS credentials from. */
  awsCredentialSource: AwsCredentialSource
  /** The access key ID, which names the key and is no secret. */
  awsAccessKeyId: string | null
  athenaWorkgroup: string | null
  athenaOutputLocation: string | null
  athenaCatalog: string | null
  /** True when Athena may give the result of an earlier run. */
  athenaResultReuse: boolean
  /** The age in minutes up to which a result may be reused. */
  athenaResultReuseMaxAgeMinutes: number
  connectionUrl: string | null
}

export interface SavedConnection {
  id: string
  name: string
  dbType: DbType
  host: string | null
  port: number | null
  user: string | null
  database: string | null
  password?: string | null
  /** The secret access key of an Athena connection. It follows the rule of
   *  the password: an absent field keeps the stored secret, and an empty
   *  text takes it away. */
  awsSecretAccessKey?: string | null
  /** The session token of an Athena connection, under the same rule. */
  awsSessionToken?: string | null
  options: ConnectionOptions
  color: string | null
  group: string | null
}

export interface DriverCapabilities {
  supportsSchemas: boolean
  supportsMultipleDatabases: boolean
  supportsCancel: boolean
  supportsTransactions: boolean
  supportsRoutines: boolean
  supportsIndexes: boolean
  supportsConstraints: boolean
  supportsPartitions: boolean
  supportsExplain: boolean
  supportsMaterializedViews: boolean
  supportsForeignTables: boolean
  supportsSynonyms: boolean
  supportsTriggers: boolean
  supportsViewTriggers: boolean
  supportsEvents: boolean
}

export interface ConnectionInfo {
  connectionId: string
  capabilities: DriverCapabilities
  dialect: Dialect
}

/**
 * What the read-only switch does on one engine. `session` makes the server
 * refuse a write, `intent` asks for a readable replica, and `none` hides the
 * switch.
 */
export type ReadOnlyMode = 'session' | 'intent' | 'none'

export interface EngineInfo {
  dbType: DbType
  label: string
  dialect: Dialect
  defaultPort: number | null
  usesHost: boolean
  usesCredentials: boolean
  usesDatabase: boolean
  usesTls: boolean
  usesFile: boolean
  usesAws: boolean
  supportsSchemas: boolean
  supportsIntegratedSecurity: boolean
  readOnly: ReadOnlyMode
}

export interface ColumnInfo {
  name: string
  typeName: string
}

export type CellValue =
  string | number | boolean | null | CellValue[] | { [key: string]: CellValue }

export interface ResultSet {
  columns: ColumnInfo[]
  rows: CellValue[][]
  truncated: boolean
}

/** What one execution cost, for an engine that reports it. */
export interface QueryStats {
  scannedBytes: number | null
  engineMs: number | null
  queueMs: number | null
  /** True when the engine gave the result of an earlier run. */
  resultReused: boolean | null
}

/** How much weight one message of a run carries. */
export const MessageLevel = {
  Info: 'info',
  Warning: 'warning',
  Error: 'error',
} as const
export type MessageLevel = (typeof MessageLevel)[keyof typeof MessageLevel]

/** One line of the Messages tab. */
export interface Message {
  level: MessageLevel
  text: string
  /** What the server said beside the text, such as a severity or a line. */
  detail: string | null
}

export interface QueryResponse {
  results: ResultSet[]
  messages: Message[]
  rowsAffected: number | null
  elapsedMs: number
  /** What the execution cost, when the engine reports it. */
  stats: QueryStats | null
}

/** What one entry of a folder is. */
export interface FolderEntry {
  name: string
  path: string
  entryType: 'folder' | 'file'
}

/** What the interface says about one command of the menu of the system. */
export interface MenuCommandState {
  id: string
  enabled: boolean
}

/** The text encoding of a file on the disk. */
export type TextEncoding = 'utf8' | 'utf8bom' | 'utf16le' | 'utf16be' | 'windows1252'

/** The encodings that the backend reads and writes. */
export const TEXT_ENCODINGS: readonly TextEncoding[] = [
  'utf8',
  'utf8bom',
  'utf16le',
  'utf16be',
  'windows1252',
]

/** The text of one file and the encoding it had on the disk. */
export interface TextFile {
  contents: string
  encoding: TextEncoding
}

/** One file that the user opened through the dialog. */
export interface OpenedFile {
  path: string
  contents: string
  /** The encoding the file had on the disk. */
  encoding?: TextEncoding
}

/** What an export to a file needs to know. */
export interface ExportRequest {
  connectionId: string
  requestId: string
  query: string
  /** The file name that the save dialog of the backend suggests. */
  defaultName: string
  format: 'csv' | 'json' | 'xlsx'
  /** The row limit of the export, which is higher than the one of the view. */
  maxRows: number
  /** The tab that runs the export, so it sees the temporary tables of the
   *  tab. */
  tabId?: string
  /** The values of the named parameters of the statement. */
  queryParams?: Record<string, unknown>
}

/** The file that the user chose for a run to a file. The run sends the
 *  ticket, and the backend writes only to the path of a ticket. */
export interface ChosenRunFile {
  ticket: string
  path: string
  /** The format that the extension of the path sets. */
  format: RunFileFormat
}

/** The formats of a run to a file. */
export type RunFileFormat = 'csv' | 'json' | 'xlsx'

/** What a run to a file sends. */
export interface RunToFileRequest {
  connectionId: string
  requestId: string
  query: string
  /** The ticket that `chooseRunFile` gave. */
  ticket: string
  /** The row limit of the file. Each result set has this limit. */
  maxRows: number
  /** True when each result set goes to the file: a CSV or JSON run writes
   *  a file for each set, and an Excel run writes a sheet for each set.
   *  False sends the first set alone. */
  eachSet?: boolean
  tabId?: string
  queryParams?: Record<string, unknown>
  /** The limits of the grid. */
  options?: ExecOptions
  /** The identifier of the file that gets every message of the run. */
  messagesFile?: string
}

/** The file that the user chose for the messages of a tab. A run sends the
 *  identifier, and the backend writes only to the path of an identifier. */
export interface ChosenMessagesFile {
  id: string
  path: string
}

/** What the window sends to save the messages that a tab shows. */
export interface SaveShownMessagesRequest {
  defaultName: string
  messages: Message[]
  /** The count of the first messages of the run that the tab dropped. */
  dropped: number
}

/**
 * One result set whose read stopped at the row limit and whose full rows the
 * backend can still read, so an export of all rows does not run the query
 * again.
 */
export interface KeptSet {
  /** The number of the set in its run, from zero. */
  set: number
  /** The identifier that `exportKept` and `releaseKept` take. */
  id: string
  /** The number of rows in the file on this computer that keeps every row
   *  of the set. Only a saved full result has this count. */
  savedRows?: number
  /** The seconds the read of the set stays paused at the row limit, for a
   *  set whose statement stays open on the server. */
  pausedSecs?: number
}

/**
 * What a run asks for when it saves its full result sets on this computer.
 * The backend reads up to `maxRows`, and every saved result together uses at
 * most `maxBytes` of disk space.
 */
export interface SpillRequest {
  maxRows: number
  maxBytes: number
}

/** What an export of a kept result needs to know. */
export interface KeptExportRequest {
  keptId: string
  /** The identifier that the Stop button of the export names. */
  requestId: string
  /** The file name that the save dialog of the backend suggests. */
  defaultName: string
  format: 'csv' | 'json' | 'xlsx'
  /** The row limit of the export, which is higher than the one of the view. */
  maxRows: number
}

/** What one export to a file wrote. */
export interface ExportSummary {
  rows: number
  /** True when even the higher row limit of the export stopped the read. */
  truncated: boolean
  /** The file the export wrote. */
  path: string
  /** True when the sheet of an xlsx file was full and rows were left out. */
  sheetFull: boolean
  /** The number of text cells of an xlsx file that were cut at the size
   *  limit of a cell. */
  cutCells: number
  /** A warning for the user about the content of the file, or null. */
  warning: string | null
}

/** What one result set of a run to a file put in its file. */
export interface SavedSet {
  path: string
  /** The sheet of the set, in an Excel file with a sheet for each set. */
  sheet: string | null
  rows: number
  /** True when the export row limit or the room of a sheet stopped the set. */
  truncated: boolean
  /** True when the sheet of the set was full and rows were left out. */
  sheetFull: boolean
}

/** What a run to a file wrote. The totals cover every file, and the path is
 *  the path of the chosen file. */
export interface RunFileSummary extends ExportSummary {
  /** Each set that went to a file, in the order of the run. */
  sets: SavedSet[]
  /** The number of sets that went to the grid alone. */
  skippedSets: number
}

/** What a request to save one file carries. The backend asks the user for
 *  the path itself. */
export interface SaveFileRequest {
  /** The file name that the save dialog suggests. */
  defaultName: string
  /** The label of the file type in the dialog. */
  filterLabel: string
  /** The extension of the file type, without the period. */
  extension: string
  /** The content: text, or base64 text for a binary file. */
  contents: string
}

/** What a request to save the statement of a tab carries. */
export interface SaveStatementRequest {
  /** The file of the tab, when the tab has one. */
  path?: string | null
  /** The file name that the save dialog suggests. */
  defaultName: string
  /** The folder the dialog opens in, when the panel contains one. */
  defaultFolder: string | null
  /** The text of the statement. */
  contents: string
  /** The encoding to write. A request without one writes UTF-8. */
  encoding?: TextEncoding
}

/** The file that a save of a statement wrote, and its encoding. */
export interface SavedStatement {
  path: string
  encoding: TextEncoding
}

export interface ExecOptions {
  maxRows: number
  timeoutSecs: number
}

export interface DatabaseRef {
  name: string
}

export interface SchemaRef {
  name: string
}

export const RelationType = {
  Table: 'table',
  View: 'view',
  MaterializedView: 'materializedView',
  PartitionedTable: 'partitionedTable',
  ForeignTable: 'foreignTable',
  Synonym: 'synonym',
} as const
export type RelationType = (typeof RelationType)[keyof typeof RelationType]

export interface TableRef {
  name: string
  relationType: RelationType
  /** The name of the object that a synonym points at. */
  target?: string
}

export const RoutineType = {
  Procedure: 'procedure',
  Function: 'function',
} as const
export type RoutineType = (typeof RoutineType)[keyof typeof RoutineType]

export interface RoutineRef {
  name: string
  routineType: RoutineType
}

export interface IndexRef {
  name: string
  columns: string[]
  unique: boolean
  primary: boolean
  /** The `INCLUDE` columns of an MS SQL Server index, which are not in the key. */
  included: string[]
}

export const ConstraintType = {
  PrimaryKey: 'primaryKey',
  ForeignKey: 'foreignKey',
  Unique: 'unique',
  Check: 'check',
  Exclusion: 'exclusion',
  Trigger: 'trigger',
  NotNull: 'notNull',
  Default: 'default',
} as const
export type ConstraintType = (typeof ConstraintType)[keyof typeof ConstraintType]

export interface ConstraintRef {
  name: string
  constraintType: ConstraintType
  columns: string[]
  detail: string | null
}

/** The time at which a trigger runs, against the change that fires it. */
export const TriggerTiming = {
  Before: 'before',
  After: 'after',
  InsteadOf: 'insteadOf',
} as const
export type TriggerTiming = (typeof TriggerTiming)[keyof typeof TriggerTiming]

/** A change that fires a trigger. */
export const TriggerEvent = {
  Insert: 'insert',
  Update: 'update',
  Delete: 'delete',
  Truncate: 'truncate',
} as const
export type TriggerEvent = (typeof TriggerEvent)[keyof typeof TriggerEvent]

export interface TriggerRef {
  name: string
  timing: TriggerTiming
  /** The changes that fire the trigger, in the order insert, update, delete and truncate. */
  events: TriggerEvent[]
  /**
   * False when the engine keeps the trigger but does not run it. A PostgreSQL
   * replica trigger is not enabled, because it runs only in a session whose
   * `session_replication_role` is `replica`.
   */
  enabled: boolean
  /** True for a PostgreSQL replica trigger. The backend leaves it out when false. */
  replica?: boolean
  /**
   * The columns of an `UPDATE OF` clause, in the order of the clause. The
   * backend leaves it out when each update fires the trigger.
   */
  updateColumns?: string[]
}

/** One scheduled event of a MySQL or MariaDB database. */
export interface EventRef {
  name: string
  enabled: boolean
  /** The schedule in the words of the engine, such as `EVERY 1 DAY`. */
  schedule?: string
}

/** The types of object, other than a relation, that the explorer can script. */
export const ObjectType = {
  Trigger: 'trigger',
  Event: 'event',
} as const
export type ObjectType = (typeof ObjectType)[keyof typeof ObjectType]

export interface PartitionRef {
  values: string
}

/** The partitions of one relation, up to the limit of the catalog read. */
export interface PartitionList {
  partitions: PartitionRef[]
  /** True when the relation holds more partitions than the list. */
  truncated: boolean
}

/** One fact about a relation, such as the number of rows it holds. */
export interface TableFact {
  name: string
  value: string
}

/** Everything the properties dialog shows about one relation. */
export interface TableDetails {
  facts: TableFact[]
  columns: ColumnRef[]
  indexes: IndexRef[]
  constraints: ConstraintRef[]
}

export interface SnapshotColumn {
  name: string
  dataType: string
}

export interface SnapshotRelation {
  name: string
  schema: string | null
  relationType: RelationType
  columns: SnapshotColumn[]
}

/**
 * Every relation and every column of one database. The editor offers these
 * names as completions, so the names of a relation the user never opened in
 * the tree are still there.
 */
export interface SchemaSnapshot {
  database: string
  relations: SnapshotRelation[]
  columnCount: number
  /** False when the bound on the columns stopped the read. */
  complete: boolean
}

/** The form of one value that the user gave for a parameter. */
export const ParamType = {
  Text: 'text',
  Number: 'number',
  Boolean: 'boolean',
  Null: 'null',
} as const
export type ParamType = (typeof ParamType)[keyof typeof ParamType]

/** One value that the user gave for a named parameter of a statement. */
export interface ParamValue {
  name: string
  valueType: ParamType
  text: string
}

/** Which plan of a statement the user asked for. */
export const PlanMode = {
  /** The plan the engine builds without running the statement. */
  Estimated: 'estimated',
  /** The plan the engine reports after it ran the statement. */
  Actual: 'actual',
} as const
export type PlanMode = (typeof PlanMode)[keyof typeof PlanMode]

/** The statement that the explorer builds for one object. */
export const ScriptStatement = {
  Create: 'create',
  Select: 'select',
  Insert: 'insert',
  Update: 'update',
} as const
export type ScriptStatement = (typeof ScriptStatement)[keyof typeof ScriptStatement]

export interface ColumnRef {
  name: string
  dataType: string
  nullable: boolean
  isPrimaryKey: boolean
  /** True when the server fills the column itself, so INSERT and UPDATE leave it out. */
  isGenerated: boolean
}

export const ConnectionHealth = {
  Connected: 'connected',
  Reconnecting: 'reconnecting',
  Disconnected: 'disconnected',
} as const
export type ConnectionHealth = (typeof ConnectionHealth)[keyof typeof ConnectionHealth]

export interface ConnectionStatusEvent {
  connectionId: string
  health: ConnectionHealth
  message: string | null
}

export interface HistoryEntry {
  id: string
  connectionId: string
  connectionName: string
  query: string
  ranAt: string
  elapsedMs: number
  rowCount: number
  succeeded: boolean
  error?: string | null
}

export const ErrorCategory = {
  NotConnected: 'notConnected',
  Connection: 'connection',
  Timeout: 'timeout',
  Cancelled: 'cancelled',
  Database: 'database',
  Configuration: 'configuration',
  Authentication: 'authentication',
  Io: 'io',
  Storage: 'storage',
  Secret: 'secret',
  Unsupported: 'unsupported',
  Invalid: 'invalid',
  Internal: 'internal',
} as const
export type ErrorCategory = (typeof ErrorCategory)[keyof typeof ErrorCategory]

/** The payload the backend sends when a command fails. */
export interface ErrorPayload {
  category: ErrorCategory
  message: string
  detail: string | null
  /** The line of the failure, from 1, in the text that was sent. */
  line?: number | null
  /** The column of the failure, from 1, on that line. */
  column?: number | null
  /** A marker for an error whose message gives its own advice, such as
   *  `kerberosUnreachable`. Null when the advice of the category applies. */
  reason?: string | null
  /** True when the tab's session closed after the failure, so the next run
   *  opens a new session. */
  sessionReset?: boolean
}

/** Builds the options that a new connection starts with. */
export function defaultConnectionOptions(): ConnectionOptions {
  return {
    tlsMode: TlsMode.VerifyFull,
    caCertPath: null,
    connectTimeoutSecs: 15,
    queryTimeoutSecs: 300,
    maxRows: 10000,
    maxSessions: 6,
    readOnly: false,
    applicationName: 'SQL Explorer',
    instanceName: null,
    integratedSecurity: false,
    mssqlAuth: MssqlAuth.SqlLogin,
    azureCliPath: null,
    filePath: null,
    awsRegion: null,
    awsProfile: null,
    awsCredentialSource: AwsCredentialSource.Chain,
    awsAccessKeyId: null,
    athenaWorkgroup: null,
    athenaOutputLocation: null,
    athenaCatalog: null,
    athenaResultReuse: false,
    athenaResultReuseMaxAgeMinutes: 60,
    connectionUrl: null,
  }
}
