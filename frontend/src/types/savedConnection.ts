export enum DbType {
  Mssql = 'Mssql',
  Mysql = 'Mysql',
  Postgres = 'Postgres'
}

export enum AuthType {
  Sql = 'Sql',
  Integrated = 'Integrated'
}

export interface SavedConnection {
  name: string
  db_type: DbType
  server: string
  database: string
  auth_type: AuthType
  user?: string
}
