import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api } from '@/lib/api'
import { useExplorerStore } from './explorer'
import { useUiStore } from './ui'
import {
  AwsCredentialSource,
  ConnectionHealth,
  DbType,
  ErrorCategory,
  MssqlAuth,
  defaultConnectionOptions,
  type ConnectionInfo,
  type ConnectionStatusEvent,
  type EngineInfo,
  type SavedConnection,
} from '@/types/api'

/** Builds a new saved connection with the defaults of its engine. */
export function newConnection(dbType: DbType = DbType.Mssql, id = createId()): SavedConnection {
  return {
    id,
    name: '',
    dbType,
    host: dbType === DbType.Sqlite || dbType === DbType.Athena ? null : 'localhost',
    port: defaultPortFor(dbType),
    user: null,
    database: null,
    password: '',
    options: defaultConnectionOptions(),
    color: null,
    group: null,
  }
}

/** Returns the port an engine listens on by default. */
export function defaultPortFor(dbType: DbType): number | null {
  switch (dbType) {
    case DbType.Mssql:
      return 1433
    case DbType.Mysql:
      return 3306
    case DbType.Postgres:
      return 5432
    default:
      return null
  }
}

/** Builds an identifier that no other record uses. */
export function createId(): string {
  const globalCrypto = globalThis.crypto as Crypto | undefined
  if (globalCrypto?.randomUUID) {
    return globalCrypto.randomUUID()
  }
  return `id-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
}

/** The most minutes that Athena reuses a result for: seven days. */
export const ATHENA_REUSE_MAX_MINUTES = 10080

/**
 * Reports the fields the record needs but does not contain, by the name of the
 * field. An empty record means that the connection is complete.
 */
export function validateConnection(connection: SavedConnection): Record<string, string> {
  const problems: Record<string, string> = {}
  if (!connection.name.trim()) {
    problems.name = 'Enter a name for the connection.'
  }
  // A number box that the user empties gives a text, and the backend
  // refuses a text where it reads a whole number.
  const { connectTimeoutSecs, queryTimeoutSecs, maxRows, maxSessions } = connection.options
  if (!isCount(connectTimeoutSecs)) {
    problems.connectTimeoutSecs = 'Connect timeout must be a whole number, 0 or greater.'
  }
  if (!isCount(queryTimeoutSecs)) {
    problems.queryTimeoutSecs = 'Statement timeout must be a whole number, 0 or greater.'
  }
  if (!isCount(maxRows)) {
    problems.maxRows = 'Row limit must be a whole number, 0 or greater.'
  }
  if (!isCount(maxSessions) || maxSessions < 1) {
    problems.maxSessions = 'Max sessions must be a whole number, 1 or greater.'
  }
  switch (connection.dbType) {
    case DbType.Sqlite:
      if (!connection.options.filePath?.trim()) {
        problems.filePath = 'Enter the path to a SQLite database file.'
      }
      break
    case DbType.Athena: {
      const options = connection.options
      if (!options.awsRegion?.trim()) {
        problems.awsRegion = 'Enter an AWS region for the Athena connection.'
      }
      if (!options.athenaWorkgroup?.trim() && !options.athenaOutputLocation?.trim()) {
        problems.athenaWorkgroup =
          'Enter a workgroup or an output location for the Athena connection.'
      }
      // The secret access key is not checked here, because the keychain can
      // already hold it and the form then shows an empty box. The backend
      // refuses an incomplete pair when the connection opens.
      if (
        options.awsCredentialSource === AwsCredentialSource.Keys &&
        !options.awsAccessKeyId?.trim()
      ) {
        problems.awsAccessKeyId = 'Enter an access key ID, or choose another credential source.'
      }
      const age = options.athenaResultReuseMaxAgeMinutes
      if (options.athenaResultReuse && (!isCount(age) || age > ATHENA_REUSE_MAX_MINUTES)) {
        problems.athenaResultReuseMaxAgeMinutes = `Max age must be a whole number of minutes from 0 to ${ATHENA_REUSE_MAX_MINUTES}.`
      }
      break
    }
    default:
      if (!connection.options.connectionUrl?.trim()) {
        if (!connection.host?.trim()) {
          problems.host = 'Enter a host for the connection.'
        }
        // An emptied number box gives a text, which is not a whole number.
        // SQL Browser gives the port of a named instance, so that port is
        // not read.
        const port: unknown = connection.port
        const browsed =
          connection.dbType === DbType.Mssql && !!connection.options.instanceName?.trim()
        if (
          !browsed &&
          (!Number.isInteger(port) || (port as number) < 1 || (port as number) > 65535)
        ) {
          problems.port = 'Port must be a whole number from 1 to 65535.'
        }
      }
  }
  return problems
}

/** Gives the first problem of a record of problems, or null for none. */
export function firstProblem(problems: Record<string, string>): string | null {
  return Object.values(problems)[0] ?? null
}

/** True for a whole number of 0 or more. */
function isCount(value: unknown): value is number {
  return Number.isInteger(value) && (value as number) >= 0
}

/** Builds the text the connection list shows below the name. */
export function connectionSubtitle(connection: SavedConnection): string {
  switch (connection.dbType) {
    case DbType.Sqlite:
      return connection.options.filePath ?? 'No file'
    case DbType.Athena:
      return (
        [connection.options.awsRegion, connection.database].filter(Boolean).join(' · ') || 'AWS'
      )
    default: {
      const host = connection.host ?? 'localhost'
      const port = connection.port
      const target = port ? `${host}:${port}` : host
      return connection.database ? `${target}/${connection.database}` : target
    }
  }
}

export const useConnectionsStore = defineStore('connections', () => {
  const ui = useUiStore()

  const saved = ref<SavedConnection[]>([])
  const engines = ref<EngineInfo[]>([])
  /** False when a saved password goes away when the application closes. */
  const passwordsPersist = ref(true)
  const active = ref<Record<string, ConnectionInfo>>({})
  const health = ref<Record<string, ConnectionHealth>>({})
  const loading = ref(false)
  const connecting = ref<Record<string, boolean>>({})
  /** Why the last connect of each connection failed, or why it was lost. */
  const lastError = ref<Record<string, string>>({})
  const testing = ref(false)

  /** The connection whose objects the explorer shows. */
  const selectedId = ref<string | null>(null)

  /**
   * The connection that refused a login while it held a pasted access token.
   * The view opens the form for this connection and asks for a new token.
   */
  const expiredTokenId = ref<string | null>(null)

  const hasActive = computed(() => Object.keys(active.value).length > 0)
  const activeList = computed(() =>
    saved.value.filter((connection) => Boolean(active.value[connection.id])),
  )
  const selected = computed(() =>
    selectedId.value ? (saved.value.find((item) => item.id === selectedId.value) ?? null) : null,
  )
  const selectedInfo = computed(() =>
    selectedId.value ? active.value[selectedId.value] : undefined,
  )

  /** The folders the list groups the connections under. */
  const groups = computed(() => {
    const names = new Set<string>()
    for (const connection of saved.value) {
      names.add(connection.group?.trim() || 'Connections')
    }
    return [...names].sort((left, right) => left.localeCompare(right))
  })

  function byId(id: string): SavedConnection | undefined {
    return saved.value.find((connection) => connection.id === id)
  }

  function isActive(id: string): boolean {
    return Boolean(active.value[id])
  }

  /**
   * The name to show for one identifier. A tab that the workspace holds can
   * name a connection that the user has since deleted, and an identifier
   * means nothing to a reader, so such a tab reports that the record is gone.
   */
  function nameFor(id: string): string {
    return byId(id)?.name ?? 'Deleted connection'
  }

  /** Reads what the connection form needs from the backend: the engines of
   *  the build, and whether a saved password stays after a restart. */
  async function loadEngines(): Promise<void> {
    try {
      const [list, persist] = await Promise.all([api.supportedEngines(), api.passwordsPersist()])
      engines.value = list
      // A backend that gives no answer keeps the keychain.
      passwordsPersist.value = persist !== false
    } catch (error) {
      ui.reportError(error)
    }
  }

  async function load(): Promise<void> {
    loading.value = true
    try {
      saved.value = await api.getConnections()
      const open = await api.listActiveConnections()
      const map: Record<string, ConnectionInfo> = {}
      // The health map is rebuilt from the connections that exist, so an
      // entry for a deleted connection does not stay behind.
      const knownHealth: Record<string, ConnectionHealth> = {}
      for (const record of saved.value) {
        const existing = health.value[record.id]
        if (existing) {
          knownHealth[record.id] = existing
        }
      }
      for (const info of open) {
        map[info.connectionId] = info
        knownHealth[info.connectionId] = ConnectionHealth.Connected
      }
      active.value = map
      health.value = knownHealth
    } catch (error) {
      ui.reportError(error)
    } finally {
      loading.value = false
    }
  }

  async function save(connection: SavedConnection): Promise<boolean> {
    const problem = firstProblem(validateConnection(connection))
    if (problem) {
      ui.warn(problem)
      return false
    }
    try {
      await api.saveConnection(connection)
      await load()
      ui.success(`Saved connection '${connection.name}'.`)
      return true
    } catch (error) {
      ui.reportError(error)
      return false
    }
  }

  async function remove(id: string): Promise<void> {
    try {
      await api.deleteConnection(id)
      delete active.value[id]
      delete health.value[id]
      setLastError(id, null)
      if (selectedId.value === id) {
        selectedId.value = null
      }
      await load()
    } catch (error) {
      ui.reportError(error)
    }
  }

  /** Records why a connection failed, or clears the record with null. */
  function setLastError(id: string, message: string | null): void {
    const rest = { ...lastError.value }
    if (message === null) {
      delete rest[id]
    } else {
      rest[id] = message
    }
    lastError.value = rest
  }

  async function connect(connection: SavedConnection): Promise<boolean> {
    // A second click while the connect runs opens no second connection.
    if (connecting.value[connection.id]) {
      return false
    }
    connecting.value = { ...connecting.value, [connection.id]: true }
    try {
      const info = await api.connect(connection.id)
      setLastError(connection.id, null)
      active.value = { ...active.value, [connection.id]: info }
      health.value = { ...health.value, [connection.id]: ConnectionHealth.Connected }
      selectedId.value = connection.id
      return true
    } catch (error) {
      const payload = ui.reportError(error)
      setLastError(connection.id, payload.message)
      // A pasted access token lives for about one hour, and the stored one
      // cannot be made fresh again. The view asks for a new token.
      if (payload.category === ErrorCategory.Authentication && usesAccessToken(connection)) {
        expiredTokenId.value = connection.id
      }
      return false
    } finally {
      const rest = { ...connecting.value }
      delete rest[connection.id]
      connecting.value = rest
    }
  }

  async function disconnect(id: string): Promise<void> {
    try {
      await api.disconnect(id)
    } catch (error) {
      // The backend may still hold the connection open, so the view keeps
      // it and the user can close it again.
      ui.reportError(error)
      return
    }
    const rest = { ...active.value }
    delete rest[id]
    active.value = rest
    health.value = { ...health.value, [id]: ConnectionHealth.Disconnected }
    if (selectedId.value === id) {
      selectedId.value = firstActiveId()
    }
  }

  async function test(connection: SavedConnection): Promise<boolean> {
    const problem = firstProblem(validateConnection(connection))
    if (problem) {
      ui.warn(problem)
      return false
    }
    testing.value = true
    try {
      const message = await api.testConnection(connection)
      ui.success(message)
      return true
    } catch (error) {
      ui.reportError(error)
      return false
    } finally {
      testing.value = false
    }
  }

  /** The identifier of the first connection that is still open. */
  function firstActiveId(): string | null {
    const first = activeList.value[0]
    return first ? first.id : null
  }

  function select(id: string | null): void {
    selectedId.value = id
  }

  /** Records a change of state that the backend reported. */
  function applyStatus(event: ConnectionStatusEvent): void {
    health.value = { ...health.value, [event.connectionId]: event.health }
    if (event.health === ConnectionHealth.Disconnected) {
      const rest = { ...active.value }
      delete rest[event.connectionId]
      active.value = rest
      if (selectedId.value === event.connectionId) {
        selectedId.value = firstActiveId()
      }
      // The tree of a dropped connection names the objects of a session that
      // is gone, so its root goes. A connect after the drop reads it again.
      useExplorerStore().removeRoot(event.connectionId)
      if (event.message) {
        // A lost connection stops the work of every tab on it, so the
        // notice stays until the user closes it.
        setLastError(event.connectionId, event.message)
        ui.reportError({
          category: ErrorCategory.NotConnected,
          message: event.message,
          detail: null,
        })
      }
    }
  }

  /** True when the connection holds a token that the user pasted. */
  function usesAccessToken(connection: SavedConnection): boolean {
    return (
      connection.dbType === DbType.Mssql &&
      connection.options.mssqlAuth === MssqlAuth.EntraAccessToken
    )
  }

  /** Takes the request for a new token away, once the view has acted on it. */
  function clearExpiredToken(): void {
    expiredTokenId.value = null
  }

  /** Builds a copy of a connection under a new name. */
  function duplicate(connection: SavedConnection): SavedConnection {
    return {
      ...connection,
      id: createId(),
      name: `${connection.name} (copy)`,
      password: '',
      options: { ...connection.options },
    }
  }

  return {
    saved,
    engines,
    passwordsPersist,
    active,
    health,
    loading,
    connecting,
    lastError,
    testing,
    selectedId,
    expiredTokenId,
    hasActive,
    activeList,
    selected,
    selectedInfo,
    groups,
    byId,
    isActive,
    nameFor,
    loadEngines,
    load,
    save,
    remove,
    connect,
    disconnect,
    test,
    select,
    applyStatus,
    duplicate,
    clearExpiredToken,
  }
})
