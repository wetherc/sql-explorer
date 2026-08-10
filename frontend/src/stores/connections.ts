import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api } from '@/lib/api'
import { useUiStore } from './ui'
import {
  ConnectionHealth,
  DbType,
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

/** Reports the fields the record needs but does not hold. */
export function validateConnection(connection: SavedConnection): string[] {
  const problems: string[] = []
  if (!connection.name.trim()) {
    problems.push('The connection needs a name.')
  }
  switch (connection.dbType) {
    case DbType.Sqlite:
      if (!connection.options.filePath?.trim()) {
        problems.push('A SQLite connection needs the path of a file.')
      }
      break
    case DbType.Athena:
      if (!connection.options.awsRegion?.trim()) {
        problems.push('An Athena connection needs an AWS region.')
      }
      if (
        !connection.options.athenaWorkgroup?.trim() &&
        !connection.options.athenaOutputLocation?.trim()
      ) {
        problems.push('An Athena connection needs a workgroup or an output location.')
      }
      break
    default:
      if (!connection.options.connectionUrl?.trim()) {
        if (!connection.host?.trim()) {
          problems.push('The connection needs a host.')
        }
        const port = connection.port
        if (port === null || !Number.isInteger(port) || port < 1 || port > 65535) {
          problems.push('The port must be a whole number between 1 and 65535.')
        }
      }
  }
  return problems
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
  const active = ref<Record<string, ConnectionInfo>>({})
  const health = ref<Record<string, ConnectionHealth>>({})
  const loading = ref(false)
  const connecting = ref<Record<string, boolean>>({})
  const testing = ref(false)

  /** The connection whose objects the explorer shows. */
  const selectedId = ref<string | null>(null)

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

  async function loadEngines(): Promise<void> {
    try {
      engines.value = await api.supportedEngines()
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
      for (const info of open) {
        map[info.connectionId] = info
        health.value[info.connectionId] = ConnectionHealth.Connected
      }
      active.value = map
    } catch (error) {
      ui.reportError(error)
    } finally {
      loading.value = false
    }
  }

  async function save(connection: SavedConnection): Promise<boolean> {
    const problems = validateConnection(connection)
    const firstProblem = problems[0]
    if (firstProblem) {
      ui.warn(firstProblem)
      return false
    }
    try {
      await api.saveConnection(connection)
      await load()
      ui.success(`The connection '${connection.name}' is saved.`)
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
      if (selectedId.value === id) {
        selectedId.value = null
      }
      await load()
    } catch (error) {
      ui.reportError(error)
    }
  }

  async function connect(connection: SavedConnection): Promise<boolean> {
    connecting.value = { ...connecting.value, [connection.id]: true }
    try {
      const info = await api.connect(connection)
      active.value = { ...active.value, [connection.id]: info }
      health.value = { ...health.value, [connection.id]: ConnectionHealth.Connected }
      selectedId.value = connection.id
      return true
    } catch (error) {
      ui.reportError(error)
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
      ui.reportError(error)
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
    const problems = validateConnection(connection)
    const firstProblem = problems[0]
    if (firstProblem) {
      ui.warn(firstProblem)
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
      if (event.message) {
        ui.warn(event.message)
      }
    }
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
    active,
    health,
    loading,
    connecting,
    testing,
    selectedId,
    hasActive,
    activeList,
    selected,
    selectedInfo,
    groups,
    byId,
    isActive,
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
  }
})
