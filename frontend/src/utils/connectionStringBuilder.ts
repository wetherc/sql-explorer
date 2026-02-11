import { AuthType, DbType } from '@/types/savedConnection'

export interface ConnectionOptions {
  dbType: DbType
  server: string
  database?: string
  authType: AuthType
  username?: string
  password?: string
  encrypt?: 'false' | 'true'
  trustServerCertificate?: boolean
  port?: number
  applicationName?: string
  connectTimeout?: number
  sslMode?: string // New property for general SSL/TLS mode
}

export function buildConnectionString(options: ConnectionOptions): string {
  if (options.dbType === DbType.Mysql) {
    return buildMysqlConnectionString(options)
  }
  if (options.dbType === DbType.Postgres) {
    return buildPostgresConnectionString(options)
  }
  return buildMssqlConnectionString(options)
}

function buildMssqlConnectionString(options: ConnectionOptions): string {
  if (!options.server) {
    throw new Error('Server name is required.')
  }

  let serverIdentifier = options.server
  if (options.port) {
    serverIdentifier += `,${options.port}`
  }

  let connectionString = `server=${serverIdentifier};`

  if (options.database) {
    connectionString += `database=${options.database};`
  }

  if (options.applicationName) {
    connectionString += `Application Name=${options.applicationName};`
  }

  if (options.connectTimeout) {
    connectionString += `Connect Timeout=${options.connectTimeout};`
  }

  if (options.encrypt) {
    connectionString += `Encrypt=${options.encrypt};`
  }

  if (options.trustServerCertificate !== undefined) {
    connectionString += `TrustServerCertificate=${options.trustServerCertificate};`
  }

  // TODO: Process options.sslMode for MSSQL connection string.
  // This will likely involve mapping values like 'Disable', 'Required' to 'Encrypt=false', 'Encrypt=true'
  // and potentially overriding trustServerCertificate based on the chosen sslMode.

  if (options.authType === AuthType.Sql) {
    if (!options.username) {
      throw new Error('Username is required for SQL Server Authentication.')
    }
    connectionString += `user=${options.username};`
    connectionString += `password=${options.password || ''};`
  } else if (options.authType === AuthType.Integrated) { // Explicitly check for Integrated
    connectionString += 'Integrated Security=true;'
  }

  return connectionString
}

function buildMysqlConnectionString(options: ConnectionOptions): string {
  if (!options.server) {
    throw new Error('Server name is required.')
  }
  if (!options.username) {
    throw new Error('Username is required for MySQL Authentication.')
  }

  const user = encodeURIComponent(options.username)
  const password = encodeURIComponent(options.password || '')
  const server = options.server
  const port = options.port || 3306
  const database = options.database || ''

  // TODO: Process options.sslMode for MySQL connection string.
  // This will likely involve mapping values like 'Disable', 'Required', 'Verify CA' to `ssl-mode=DISABLED`, `ssl-mode=REQUIRED`, `ssl-mode=VERIFY_CA` in the URL.
  let queryString = '';
  if (options.sslMode) {
    queryString += `ssl-mode=${options.sslMode}`; // Placeholder, actual mapping will be more complex
  }

  const finalDatabase = database ? `/${database}` : '';
  const finalQueryString = queryString ? `?${queryString}` : '';

  return `mysql://${user}:${password}@${server}:${port}${finalDatabase}${finalQueryString}`
}

function buildPostgresConnectionString(options: ConnectionOptions): string {
  if (!options.server) {
    throw new Error('Server name is required.')
  }
  if (!options.username) {
    throw new Error('Username is required for PostgreSQL Authentication.')
  }

  const user = encodeURIComponent(options.username)
  const password = encodeURIComponent(options.password || '')
  const server = options.server
  const port = options.port || 5432
  const database = options.database || ''

  // TODO: Process options.sslMode for PostgreSQL connection string.
  // This will likely involve mapping values like 'Disable', 'Require', 'Verify CA' to `sslmode=disable`, `sslmode=require`, `sslmode=verify-ca` in the URL.
  let queryString = '';
  if (options.sslMode) {
    queryString += `sslmode=${options.sslMode}`; // Placeholder, actual mapping will be more complex
  }

  const finalDatabase = database ? `/${database}` : '';
  const finalQueryString = queryString ? `?${queryString}` : '';

  return `postgresql://${user}:${password}@${server}:${port}${finalDatabase}${finalQueryString}`
}
