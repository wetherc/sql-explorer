export enum AuthType {
  Sql = 'Sql',
  Integrated = 'Integrated'
}

export interface SavedConnection {
  name: string
  server: string
  database: string
  auth_type: AuthType
  user?: string
}
