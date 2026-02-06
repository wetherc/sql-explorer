export type AuthType = 'sql' | 'integrated'

export interface ConnectionOptions {
  server: string
  database?: string
  authType: AuthType
  username?: string
  password?: string
}

export function buildConnectionString(options: ConnectionOptions): string {
  if (!options.server) {
    throw new Error('Server name is required.')
  }

  let connectionString = `server=${options.server};`

  if (options.database) {
    connectionString += `database=${options.database};`
  }

  // Always include this for modern SQL Server versions
  connectionString += 'TrustServerCertificate=true;'

  if (options.authType === 'sql') {
    if (!options.username) {
      throw new Error('Username is required for SQL Server Authentication.')
    }
    connectionString += `user=${options.username};`
    connectionString += `password=${options.password || ''};`
  } else {
    connectionString += 'Authentication=ActiveDirectoryIntegrated;'
  }

  return connectionString
}
