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

export interface ConnectionOptions {
  tlsMode: TlsMode
  caCertPath: string | null
  connectTimeoutSecs: number
  queryTimeoutSecs: number
  maxRows: number
  readOnly: boolean
  applicationName: string | null
  instanceName: string | null
  integratedSecurity: boolean
  filePath: string | null
  awsRegion: string | null
  awsProfile: string | null
  athenaWorkgroup: string | null
  athenaOutputLocation: string | null
  athenaCatalog: string | null
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
  options: ConnectionOptions
  color: string | null
  group: string | null
}

export interface DriverCapabilities {
  supportsSchemas: boolean
  supportsMultipleDatabases: boolean
  supportsCancel: boolean
  supportsTransactions: boolean
}

export interface ConnectionInfo {
  connectionId: string
  capabilities: DriverCapabilities
  dialect: Dialect
}

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

export interface QueryResponse {
  results: ResultSet[]
  messages: string[]
  rowsAffected: number | null
  elapsedMs: number
}

/** What an export to a file needs to know. */
export interface ExportRequest {
  connectionId: string
  requestId: string
  query: string
  path: string
  format: 'csv' | 'json'
  /** The row limit of the export, which is higher than the one of the view. */
  maxRows: number
}

/** What one export to a file wrote. */
export interface ExportSummary {
  rows: number
  /** True when even the higher row limit of the export stopped the read. */
  truncated: boolean
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

export const TableKind = {
  Table: 'table',
  View: 'view',
} as const
export type TableKind = (typeof TableKind)[keyof typeof TableKind]

export interface TableRef {
  name: string
  kind: TableKind
}

export interface ColumnRef {
  name: string
  dataType: string
  nullable: boolean
  isPrimaryKey: boolean
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

export interface SavedQuery {
  id: string
  name: string
  query: string
  connectionId?: string | null
  folder?: string | null
  updatedAt: string
}

export const ErrorKind = {
  NotConnected: 'notConnected',
  Connection: 'connection',
  Timeout: 'timeout',
  Cancelled: 'cancelled',
  Database: 'database',
  Configuration: 'configuration',
  Io: 'io',
  Storage: 'storage',
  Secret: 'secret',
  Unsupported: 'unsupported',
  Internal: 'internal',
} as const
export type ErrorKind = (typeof ErrorKind)[keyof typeof ErrorKind]

/** The payload the backend sends when a command fails. */
export interface ErrorPayload {
  kind: ErrorKind
  message: string
  detail: string | null
}

/** Builds the options that a new connection starts with. */
export function defaultConnectionOptions(): ConnectionOptions {
  return {
    tlsMode: TlsMode.VerifyFull,
    caCertPath: null,
    connectTimeoutSecs: 15,
    queryTimeoutSecs: 300,
    maxRows: 10000,
    readOnly: false,
    applicationName: 'SQL Explorer',
    instanceName: null,
    integratedSecurity: false,
    filePath: null,
    awsRegion: null,
    awsProfile: null,
    athenaWorkgroup: null,
    athenaOutputLocation: null,
    athenaCatalog: null,
    connectionUrl: null,
  }
}
