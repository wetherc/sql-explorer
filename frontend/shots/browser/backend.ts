/**
 * Returns the sample data for each command of the interface, in place of the
 * Rust backend. The screenshots run the real interface in a browser, and
 * `mockIPC` sends each command here.
 */
import { ErrorCategory, type ConnectionInfo, type ErrorPayload } from '@/types/api'
import { runFrames } from './frames'
import {
  ACTIVE,
  CAPABILITIES,
  CATALOG,
  CONNECTIONS,
  ENGINES,
  HISTORY,
  PLAN_LINES,
  RUNS,
  WORKSPACE,
  type SampleRelation,
} from './fixtures'

type Args = Record<string, unknown>

/** The receiving end of a channel, which `mockIPC` passes as it is. */
interface ChunkChannel {
  onmessage: (message: ArrayBuffer) => void
}

/** The error that a command of the real backend throws. */
function failure(category: ErrorCategory, message: string): ErrorPayload {
  return { category, message, detail: null }
}

function infoFor(connectionId: string): ConnectionInfo {
  const saved = CONNECTIONS.find((item) => item.id === connectionId)
  if (!saved) {
    throw failure(ErrorCategory.NotConnected, `No connection has the identifier ${connectionId}.`)
  }
  const engine = ENGINES.find((item) => item.dbType === saved.dbType)!
  return { connectionId, capabilities: CAPABILITIES[saved.dbType], dialect: engine.dialect }
}

/** The fields of a command that takes one `request` record. */
function fields(args: Args): Args {
  return args.request as Args
}

function schemaOf(args: Args) {
  const { connectionId, database, schemaName } = fields(args) as {
    connectionId: string
    database: string
    schemaName: string | null
  }
  const found = CATALOG[connectionId]?.find((item) => item.name === database)?.schemas[
    schemaName ?? ''
  ]
  return found ?? { relations: [], routines: [] }
}

function relationOf(args: Args): SampleRelation {
  const { tableName } = fields(args) as { tableName: string }
  const relation = schemaOf(args).relations.find((item) => item.name === tableName)
  if (!relation) {
    throw failure(ErrorCategory.Database, `The sample catalog has no relation ${tableName}.`)
  }
  return relation
}

/** The names after a colon, in the order of their first place. */
export function parameterNames(query: string): string[] {
  const names = [...query.matchAll(/(?<![:\w]):([A-Za-z_]\w*)/g)].map((match) => match[1]!)
  return [...new Set(names)]
}

/** Builds the function that `mockIPC` calls with each command. Each call of
 *  `createBackend` starts from the same sample history and workspace. */
export function createBackend(): (command: string, args?: Args) => unknown {
  let workspace: unknown = structuredClone(WORKSPACE)
  let history = structuredClone(HISTORY)

  const commands: Record<string, (args: Args) => unknown> = {
    supported_engines: () => ENGINES,
    passwords_persist: () => true,
    storage_problems: () => [],
    set_menu_commands: () => null,
    get_connections: () => CONNECTIONS,
    list_active_connections: () => ACTIVE.map(infoFor),
    connect: (args) => infoFor(args.connectionId as string),
    disconnect: () => null,
    test_connection: () => 'Connected.',
    release_session: () => null,
    cancel_query: () => null,
    get_history: () => history,
    add_history_entry: (args) => {
      history = [args.entry as (typeof history)[number], ...history]
      return null
    },
    clear_history: () => {
      history = []
      return null
    },
    get_workspace: () => workspace,
    save_workspace: (args) => {
      workspace = args.workspace
      return null
    },
    file_roots: () => [],
    pick_folder: () => null,
    open_statement_file: () => null,
    save_statement_file: () => null,
    export_query: () => null,
    list_databases: (args) =>
      (CATALOG[args.connectionId as string] ?? []).map((item) => ({ name: item.name })),
    list_schemas: (args) =>
      Object.keys(
        CATALOG[args.connectionId as string]?.find((item) => item.name === args.database)
          ?.schemas ?? {},
      ).map((name) => ({ name })),
    list_tables: (args) =>
      schemaOf(args).relations.map(({ name, relationType }) => ({ name, relationType })),
    list_routines: (args) => schemaOf(args).routines ?? [],
    list_events: () => [],
    list_columns: (args) => relationOf(args).columns,
    list_indexes: (args) => relationOf(args).indexes ?? [],
    list_constraints: (args) => relationOf(args).constraints ?? [],
    list_triggers: () => [],
    list_partitions: () => ({ partitions: [], truncated: false }),
    table_details: (args) => {
      const relation = relationOf(args)
      return {
        facts: relation.facts ?? [],
        columns: relation.columns,
        indexes: relation.indexes ?? [],
        constraints: relation.constraints ?? [],
      }
    },
    schema_snapshot: (args) => {
      const { connectionId, database } = fields(args) as { connectionId: string; database: string }
      const schemas = CATALOG[connectionId]?.find((item) => item.name === database)?.schemas ?? {}
      const relations = Object.entries(schemas).flatMap(([schema, content]) =>
        content.relations.map((relation) => ({
          name: relation.name,
          schema: schema === '' ? null : schema,
          relationType: relation.relationType,
          columns: relation.columns.map(({ name, dataType }) => ({ name, dataType })),
        })),
      )
      const columnCount = relations.reduce((sum, relation) => sum + relation.columns.length, 0)
      return { database, relations, columnCount, complete: true }
    },
    quote_identifier: (args) => `"${args.name as string}"`,
    preview_query: (args) => {
      const { schemaName, tableName } = fields(args) as { schemaName: string; tableName: string }
      return `select * from ${schemaName}.${tableName} limit 1000;`
    },
    query_parameters: (args) => parameterNames(args.query as string),
    execute_query: (args) => {
      const { query } = fields(args) as { query: string }
      const run = RUNS.find((item) => item.matches(query))
      if (!run) {
        throw failure(
          ErrorCategory.Database,
          'The screenshot backend has no rows for this statement.',
        )
      }
      const end = {
        messages: [],
        rowsAffected: null,
        elapsedMs: run.elapsedMs,
        stats: run.stats ?? null,
      }
      ;(args.onChunk as ChunkChannel).onmessage(runFrames(run.sets, [], end))
      return null
    },
    explain_query: () => ({
      results: [
        {
          columns: [{ name: 'QUERY PLAN', typeName: 'text' }],
          rows: PLAN_LINES.map((line) => [line]),
          truncated: false,
        },
      ],
      messages: [],
      rowsAffected: null,
      elapsedMs: 24,
      stats: null,
    }),
  }

  return (command, args = {}) => {
    const handler = commands[command]
    if (!handler) {
      throw failure(
        ErrorCategory.Unsupported,
        `The screenshot backend has no handler for the command ${command}.`,
      )
    }
    return handler(args)
  }
}
