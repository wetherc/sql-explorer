// src/types/db.ts
export const DbType = {
  Mssql: 'Mssql',
  Mysql: 'Mysql',
  Postgres: 'Postgres',
} as const;

export type DbType = typeof DbType[keyof typeof DbType];
