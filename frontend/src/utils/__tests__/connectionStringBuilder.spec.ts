import { describe, it, expect } from 'vitest'
import { buildConnectionString } from '../connectionStringBuilder'

describe('connectionStringBuilder', () => {
  it('builds a correct SQL Server Auth string', () => {
    const result = buildConnectionString({
      server: 'my-server',
      database: 'my-db',
      authType: 'sql',
      username: 'my-user',
      password: 'my-password',
    })
    expect(result).toBe('server=my-server;database=my-db;TrustServerCertificate=true;user=my-user;password=my-password;')
  })

  it('builds a correct Integrated Auth string', () => {
    const result = buildConnectionString({
      server: 'my-server',
      database: 'my-db',
      authType: 'integrated',
    })
    expect(result).toBe('server=my-server;database=my-db;TrustServerCertificate=true;Authentication=ActiveDirectoryIntegrated;')
  })

  it('handles an empty database name', () => {
    const result = buildConnectionString({
      server: 'my-server',
      authType: 'integrated',
    })
    expect(result).toBe('server=my-server;TrustServerCertificate=true;Authentication=ActiveDirectoryIntegrated;')
  })

  it('handles an empty password for SQL auth', () => {
    const result = buildConnectionString({
      server: 'my-server',
      authType: 'sql',
      username: 'my-user',
    })
    expect(result).toBe('server=my-server;TrustServerCertificate=true;user=my-user;password=;')
  })

  it('throws an error if server is missing', () => {
    expect(() => buildConnectionString({
      server: '',
      authType: 'sql',
      username: 'user',
    })).toThrow('Server name is required.')
  })

  it('throws an error if username is missing for SQL auth', () => {
    expect(() => buildConnectionString({
      server: 'my-server',
      authType: 'sql',
    })).toThrow('Username is required for SQL Server Authentication.')
  })
})
