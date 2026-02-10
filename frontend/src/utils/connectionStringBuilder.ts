import { DbType } from '@/types/savedConnection'

export type AuthType = 'sql' | 'integrated'

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
}

export function buildConnectionString(options: ConnectionOptions): string {
  if (options.dbType === DbType.Mysql) {
    return buildMysqlConnectionString(options)
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

  if (options.authType === 'sql') {
    if (!options.username) {
      throw new Error('Username is required for SQL Server Authentication.')
    }
    connectionString += `user=${options.username};`
    connectionString += `password=${options.password || ''};`
  } else {
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

  return `mysql://${user}:${password}@${server}:${port}/${database}`
}
