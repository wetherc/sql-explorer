import { vi } from 'vitest'
import type { SavedConnection } from '@/types/api'
import { DbType, Dialect, defaultConnectionOptions } from '@/types/api'

/** A stub for every backend command, so no test reaches a real bridge. */
export function makeApiStub() {
  return {
    connect: vi.fn(),
    testConnection: vi.fn(),
    disconnect: vi.fn(),
    listActiveConnections: vi.fn(),
    executeQuery: vi.fn(),
    explainQuery: vi.fn(),
    queryParameters: vi.fn(),
    cancelQuery: vi.fn(),
    listDatabases: vi.fn(),
    listSchemas: vi.fn(),
    listTables: vi.fn(),
    listColumns: vi.fn(),
    previewQuery: vi.fn(),
    listRoutines: vi.fn(),
    listIndexes: vi.fn(),
    listConstraints: vi.fn(),
    listPartitions: vi.fn(),
    schemaSnapshot: vi.fn(),
    tableDetails: vi.fn(),
    scriptObject: vi.fn(),
    quoteIdentifier: vi.fn(),
    getConnections: vi.fn(),
    saveConnection: vi.fn(),
    deleteConnection: vi.fn(),
    getHistory: vi.fn(),
    addHistoryEntry: vi.fn(),
    clearHistory: vi.fn(),
    getSavedQueries: vi.fn(),
    saveQuery: vi.fn(),
    deleteSavedQuery: vi.fn(),
    getWorkspace: vi.fn(),
    saveWorkspace: vi.fn(),
    writeTextFile: vi.fn(),
    writeBinaryFile: vi.fn(),
    exportQuery: vi.fn(),
    readTextFile: vi.fn(),
    supportedEngines: vi.fn(),
    onConnectionStatus: vi.fn(),
  }
}

/** Builds a saved connection for a test. */
export function connectionFixture(overrides: Partial<SavedConnection> = {}): SavedConnection {
  return {
    id: 'c1',
    name: 'Server',
    dbType: DbType.Mssql,
    host: 'localhost',
    port: 1433,
    user: 'sa',
    database: 'Sales',
    password: 'secret',
    options: defaultConnectionOptions(),
    color: null,
    group: null,
    ...overrides,
  }
}

/** Builds the record the backend returns after a connection opens. */
export function infoFixture(connectionId = 'c1', supportsSchemas = true) {
  return {
    connectionId,
    capabilities: {
      supportsSchemas,
      supportsMultipleDatabases: true,
      supportsCancel: true,
      supportsTransactions: true,
      supportsRoutines: true,
      supportsIndexes: true,
      supportsConstraints: true,
      supportsPartitions: false,
      supportsExplain: true,
    },
    dialect: Dialect.MsSql,
  }
}
