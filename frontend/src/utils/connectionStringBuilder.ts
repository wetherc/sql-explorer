
export type AuthType = 'sql' | 'integrated'

export interface ConnectionOptions {
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
