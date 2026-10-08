import { describe, expect, it } from 'vitest'
import { ResultStream, type ResultTable, type RunEnd } from '@/lib/results'
import { createBackend, parameterNames } from '../backend'
import { ACTIVE, CONNECTIONS, ENGINES, HISTORY, PLAN_LINES, WORKSPACE } from '../fixtures'

const shop = { connectionId: 'shop', database: 'shop', schemaName: 'public' }
const orders = { ...shop, tableName: 'orders' }

/** Runs a statement and gives back the sets and the end of the run. */
function execute(backend: ReturnType<typeof createBackend>, query: string) {
  const sets: ResultTable[] = []
  let end: RunEnd | null = null
  const stream = new ResultStream({
    onSet: (table) => sets.push(table),
    onEnd: (value) => (end = value),
  })
  backend('execute_query', {
    request: { connectionId: 'shop', requestId: 'r', query },
    onChunk: { onmessage: (buffer: ArrayBuffer) => stream.feed(buffer) },
  })
  return { sets, end: end as RunEnd | null }
}

describe('parameterNames', () => {
  it('gives each name once, in order, and skips a cast', () => {
    expect(parameterNames('select :b, :a, :b, x::int, y:z')).toEqual(['b', 'a'])
  })
})

describe('createBackend', () => {
  it('returns the sample data of the commands that run when the window opens', () => {
    const backend = createBackend()
    expect(backend('supported_engines')).toBe(ENGINES)
    expect(backend('passwords_persist', {})).toBe(true)
    expect(backend('storage_problems', {})).toEqual([])
    expect(backend('set_menu_commands', {})).toBeNull()
    expect(backend('get_connections', {})).toBe(CONNECTIONS)
    expect(backend('file_roots', {})).toEqual([])
    const active = backend('list_active_connections', {}) as Array<{ connectionId: string }>
    expect(active.map((info) => info.connectionId)).toEqual(ACTIVE)
    expect(backend('get_workspace', {})).toEqual(WORKSPACE)
    expect(backend('get_history', {})).toEqual(HISTORY)
  })

  it('returns null or an empty list for the commands that change no sample data', () => {
    const backend = createBackend()
    for (const command of [
      'disconnect',
      'release_session',
      'cancel_query',
      'pick_folder',
      'open_statement_file',
      'save_statement_file',
      'export_query',
    ]) {
      expect(backend(command, {})).toBeNull()
    }
    expect(backend('test_connection', {})).toBe('Connected.')
    expect(backend('list_events', {})).toEqual([])
    expect(backend('list_triggers', {})).toEqual([])
    expect(backend('list_partitions', {})).toEqual({ partitions: [], truncated: false })
  })

  it('connects a known connection and refuses an unknown one', () => {
    const backend = createBackend()
    expect(backend('connect', { connectionId: 'warehouse' })).toMatchObject({
      connectionId: 'warehouse',
      dialect: 'msSql',
    })
    expect(() => backend('connect', { connectionId: 'nope' })).toThrow()
    try {
      backend('connect', { connectionId: 'nope' })
    } catch (error) {
      expect(error).toMatchObject({ category: 'notConnected' })
    }
  })

  it('keeps the history and the workspace that the interface writes', () => {
    const backend = createBackend()
    const entry = { ...HISTORY[0]!, id: 'new' }
    backend('add_history_entry', { entry })
    expect((backend('get_history', {}) as unknown[])[0]).toEqual(entry)
    backend('clear_history', {})
    expect(backend('get_history', {})).toEqual([])
    backend('save_workspace', { workspace: { tabs: [] } })
    expect(backend('get_workspace', {})).toEqual({ tabs: [] })
    // A new backend starts from the samples again.
    expect(createBackend()('get_history', {})).toEqual(HISTORY)
  })

  it('reads the catalog of the tree', () => {
    const backend = createBackend()
    expect(backend('list_databases', { connectionId: 'shop' })).toEqual([
      { name: 'shop' },
      { name: 'shop_staging' },
    ])
    expect(backend('list_databases', { connectionId: 'nope' })).toEqual([])
    expect(backend('list_schemas', { connectionId: 'shop', database: 'shop' })).toEqual([
      { name: 'public' },
      { name: 'sales' },
      { name: 'staging' },
    ])
    expect(backend('list_schemas', { connectionId: 'nope', database: 'x' })).toEqual([])
    const tables = backend('list_tables', { request: shop }) as Array<{ name: string }>
    expect(tables.map((table) => table.name)).toContain('orders')
    expect(tables[0]).toEqual({ name: 'customers', relationType: 'table' })
    const lake = { connectionId: 'lake', database: 'lake', schemaName: null }
    expect(backend('list_tables', { request: lake })).toHaveLength(2)
    expect(backend('list_tables', { request: { ...shop, database: 'gone' } })).toEqual([])
    expect(backend('list_routines', { request: shop })).toHaveLength(2)
    expect(backend('list_routines', { request: { ...shop, schemaName: 'sales' } })).toEqual([])
    expect(backend('list_routines', { request: { ...shop, database: 'gone' } })).toEqual([])
  })

  it('reads the parts of a relation, and an empty list for a part it lacks', () => {
    const backend = createBackend()
    expect(backend('list_columns', { request: orders })).toHaveLength(8)
    expect(backend('list_indexes', { request: orders })).toHaveLength(3)
    expect(backend('list_constraints', { request: orders })).toHaveLength(3)
    expect(backend('table_details', { request: orders })).toMatchObject({
      facts: [{ name: 'Rows' }, { name: 'Size' }, { name: 'Owner' }],
    })
    const customers = { ...shop, tableName: 'customers' }
    expect(backend('list_indexes', { request: customers })).toEqual([])
    expect(backend('list_constraints', { request: customers })).toEqual([])
    expect(backend('table_details', { request: customers })).toMatchObject({
      facts: [],
      indexes: [],
      constraints: [],
    })
    expect(() => backend('list_columns', { request: { ...shop, tableName: 'gone' } })).toThrow()
  })

  it('reads the schema of a database for the completions', () => {
    const backend = createBackend()
    const snapshot = backend('schema_snapshot', {
      request: { connectionId: 'shop', database: 'shop' },
    }) as { relations: Array<{ schema: string | null }>; columnCount: number }
    expect(snapshot.relations[0]!.schema).toBe('public')
    expect(snapshot.columnCount).toBeGreaterThan(0)
    const lake = backend('schema_snapshot', {
      request: { connectionId: 'lake', database: 'lake' },
    }) as { relations: Array<{ schema: string | null }> }
    expect(lake.relations[0]!.schema).toBeNull()
    expect(
      backend('schema_snapshot', { request: { connectionId: 'nope', database: 'x' } }),
    ).toEqual({ database: 'x', relations: [], columnCount: 0, complete: true })
  })

  it('builds the small statements of the explorer', () => {
    const backend = createBackend()
    expect(backend('quote_identifier', { name: 'orders' })).toBe('"orders"')
    expect(backend('preview_query', { request: orders })).toBe(
      'select * from public.orders limit 1000;',
    )
    expect(backend('query_parameters', { query: 'where a = :a' })).toEqual(['a'])
  })

  it('sends the rows of a known statement through the channel', () => {
    const backend = createBackend()
    const orders = execute(backend, WORKSPACE.tabs[0]!.query)
    expect(orders.sets[0]!.rowCount).toBe(50)
    expect(orders.end).toMatchObject({ elapsedMs: 132, stats: null })
    const athena = execute(backend, WORKSPACE.tabs[2]!.query)
    expect(athena.sets[0]!.rowCount).toBe(9)
    expect(athena.end!.stats).toMatchObject({ scannedBytes: 1_530_000_000 })
  })

  it('refuses a statement that has no sample rows', () => {
    expect(() => execute(createBackend(), 'select 1')).toThrow()
  })

  it('gives the plan of a statement as rows of text', () => {
    const plan = createBackend()('explain_query', {}) as { results: Array<{ rows: unknown[] }> }
    expect(plan.results[0]!.rows).toEqual(PLAN_LINES.map((line) => [line]))
  })

  it('refuses a command it does not know', () => {
    expect(() => createBackend()('drop_everything', {})).toThrow(/drop_everything/)
  })
})
