import { vi } from 'vitest'
import { ResultTable, type ResultStreamHandlers } from '@/lib/results'
import type { CellValue, ColumnInfo, Message, QueryStats, SavedConnection } from '@/types/api'
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
    releaseSession: vi.fn(),
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
    pickFolder: vi.fn(),
    restoreFolder: vi.fn(),
    listFolder: vi.fn(),
    readTextFile: vi.fn(),
    writeTextFile: vi.fn(),
    saveStatementFile: vi.fn(),
    saveTextFile: vi.fn(),
    saveBinaryFile: vi.fn(),
    exportQuery: vi.fn(),
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

/** One answer of a run, in the shape a test writes it. */
export interface ResponseFixture {
  results: Array<{ columns?: ColumnInfo[]; rows: CellValue[][]; truncated?: boolean }>
  messages?: Message[]
  rowsAffected?: number | null
  elapsedMs?: number
  stats?: QueryStats | null
}

/**
 * Builds the answer of the stub of `executeQuery`. The command sends its rows
 * on a channel, so the stub reports each set and then the numbers of the run,
 * as the backend does.
 */
export function streamed(response: ResponseFixture) {
  return async (_request: unknown, handlers: ResultStreamHandlers): Promise<void> => {
    for (const result of response.results) {
      handlers.onSet(
        ResultTable.fromRows(result.columns ?? [], result.rows, result.truncated ?? false),
      )
    }
    handlers.onEnd({
      messages: response.messages ?? [],
      rowsAffected: response.rowsAffected ?? null,
      elapsedMs: response.elapsedMs ?? 0,
      stats: response.stats ?? null,
    })
  }
}
