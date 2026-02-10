import { describe, it, expect } from 'vitest'
import { buildConnectionString } from '../connectionStringBuilder'
import { DbType } from '@/types/savedConnection'

describe('connectionStringBuilder', () => {
  it('builds a correct SQL Server Auth string', () => {
    const result = buildConnectionString({
      dbType: DbType.Mssql,
      server: 'my-server',
      database: 'my-db',
      authType: 'sql',
      username: 'my-user',
      password: 'my-password'
    })
    expect(result).toBe('server=my-server;database=my-db;user=my-user;password=my-password;')
  })

  it('builds a correct Integrated Auth string', () => {
    const result = buildConnectionString({
      dbType: DbType.Mssql,
      server: 'my-server',
      database: 'my-db',
      authType: 'integrated'
    })
    expect(result).toBe('server=my-server;database=my-db;Integrated Security=true;')
  })

  it('handles an empty database name', () => {
    const result = buildConnectionString({
      dbType: DbType.Mssql,
      server: 'my-server',
      authType: 'integrated'
    })
    expect(result).toBe('server=my-server;Integrated Security=true;')
  })

  it('handles an empty password for SQL auth', () => {
    const result = buildConnectionString({
      dbType: DbType.Mssql,
      server: 'my-server',
      authType: 'sql',
      username: 'my-user'
    })
    expect(result).toBe('server=my-server;user=my-user;password=;')
  })

  it('throws an error if server is missing for mssql', () => {
    expect(() =>
      buildConnectionString({
        dbType: DbType.Mssql,
        server: '',
        authType: 'sql',
        username: 'user'
      })
    ).toThrow('Server name is required.')
  })

  it('throws an error if username is missing for SQL auth', () => {
    expect(() =>
      buildConnectionString({
        dbType: DbType.Mssql,
        server: 'my-server',
        authType: 'sql'
      })
    ).toThrow('Username is required for SQL Server Authentication.')
  })

  // MySQL tests
  it('builds a correct MySQL connection string', () => {
    const result = buildConnectionString({
      dbType: DbType.Mysql,
      server: 'my-server',
      database: 'my-db',
      authType: 'sql',
      username: 'my-user',
      password: 'my-password'
    })
    expect(result).toBe('mysql://my-user:my-password@my-server:3306/my-db')
  })

  it('builds a correct MySQL connection string with custom port', () => {
    const result = buildConnectionString({
      dbType: DbType.Mysql,
      server: 'my-server',
      port: 3307,
      database: 'my-db',
      authType: 'sql',
      username: 'my-user',
      password: 'my-password'
    })
    expect(result).toBe('mysql://my-user:my-password@my-server:3307/my-db')
  })

  it('throws an error if server is missing for mysql', () => {
    expect(() =>
      buildConnectionString({
        dbType: DbType.Mysql,
        server: '',
        authType: 'sql',
        username: 'user'
      })
    ).toThrow('Server name is required.')
  })

  it('throws an error if username is missing for mysql', () => {
    expect(() =>
      buildConnectionString({
        dbType: DbType.Mysql,
        server: 'my-server',
        authType: 'sql'
      })
    ).toThrow('Username is required for MySQL Authentication.')
  })

  // PostgreSQL tests
  it('builds a correct PostgreSQL connection string', () => {
    const result = buildConnectionString({
      dbType: DbType.Postgres,
      server: 'my-server',
      database: 'my-db',
      authType: 'sql',
      username: 'my-user',
      password: 'my-password'
    })
    expect(result).toBe('postgresql://my-user:my-password@my-server:5432/my-db')
  })
})
