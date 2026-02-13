// src/types/db.ts
export const DbType = {
  Mssql: 'mssql',
  Mysql: 'mysql',
  Postgres: 'postgres',
} as const;

export type DbType = typeof DbType[keyof typeof DbType];
