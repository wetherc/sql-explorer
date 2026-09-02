import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api } from '@/lib/api'
import { parseParamValues } from '@/lib/params'
import type { ParamValue } from '@/types/api'
import { useConnectionsStore } from './connections'
import { createId } from './connections'
import { useQueryStore } from './query'

export interface QueryTab {
  id: string
  title: string
  query: string
  connectionId: string | null
  /** True when the text differs from the saved statement it came from. */
  dirty: boolean
  /** The saved statement this tab came from, when it came from one. */
  savedQueryId: string | null
  /** The values the user gave for the named parameters of the statement. */
  params: ParamValue[]
  /** The file on the disk this tab came from, when it came from one. */
  filePath: string | null
}

/** The shape the open tabs take in the workspace file. */
export interface Workspace {
  tabs: Array<
    Pick<
      QueryTab,
      'id' | 'title' | 'query' | 'connectionId' | 'dirty' | 'savedQueryId' | 'params' | 'filePath'
    >
  >
  activeTabId: string | null
}

/** Reads a workspace record and drops anything that is not usable. */
export function parseWorkspace(value: unknown): Workspace {
  const empty: Workspace = { tabs: [], activeTabId: null }
  if (typeof value !== 'object' || value === null) {
    return empty
  }
  const record = value as Record<string, unknown>
  if (!Array.isArray(record.tabs)) {
    return empty
  }
  const tabs = record.tabs
    .filter((tab): tab is Record<string, unknown> => typeof tab === 'object' && tab !== null)
    .filter((tab) => typeof tab.id === 'string' && typeof tab.query === 'string')
    .map((tab) => ({
      id: tab.id as string,
      title: typeof tab.title === 'string' ? tab.title : 'Query',
      query: tab.query as string,
      connectionId: typeof tab.connectionId === 'string' ? tab.connectionId : null,
      dirty: tab.dirty === true,
      savedQueryId: typeof tab.savedQueryId === 'string' ? tab.savedQueryId : null,
      params: parseParamValues(tab.params),
      filePath: typeof tab.filePath === 'string' && tab.filePath !== '' ? tab.filePath : null,
    }))
  const activeTabId =
    typeof record.activeTabId === 'string' && tabs.some((tab) => tab.id === record.activeTabId)
      ? record.activeTabId
      : (tabs[0]?.id ?? null)
  return { tabs, activeTabId }
}

export const useTabsStore = defineStore('tabs', () => {
  const connections = useConnectionsStore()

  const tabs = ref<QueryTab[]>([])
  const activeTabId = ref<string | null>(null)
  let counter = 0
  /**
   * The count of the changes of the workspace. Every change of a tab raises
   * it, so a watcher of the workspace file follows one number and does not
   * walk each record of each tab on every keystroke.
   */
  const revision = ref(0)

  /** Records that the workspace record changed. */
  function changed(): void {
    revision.value += 1
  }

  const activeTab = computed(() => tabs.value.find((tab) => tab.id === activeTabId.value) ?? null)
  const hasTabs = computed(() => tabs.value.length > 0)

  function nextTitle(): string {
    counter += 1
    return `Query ${counter}`
  }

  function add(
    options: {
      connectionId?: string | null
      query?: string
      title?: string
      filePath?: string | null
    } = {},
  ): QueryTab {
    const tab: QueryTab = {
      id: createId(),
      title: options.title ?? nextTitle(),
      query: options.query ?? '',
      connectionId: options.connectionId ?? connections.selectedId,
      dirty: false,
      savedQueryId: null,
      params: [],
      filePath: options.filePath ?? null,
    }
    tabs.value = [...tabs.value, tab]
    activeTabId.value = tab.id
    changed()
    return tab
  }

  /**
   * Gives the session of one tab back to the backend. A statement that
   * still runs is stopped first, so the session does not run for a tab
   * that is gone. The release itself is not awaited: a session that the
   * call misses closes with the idle reap of the backend.
   */
  function releaseSession(tab: QueryTab, connectionId: string | null = tab.connectionId): void {
    if (!connectionId) {
      return
    }
    const queries = useQueryStore()
    void queries
      .cancel(tab.id)
      .then(() => api.releaseSession(connectionId, tab.id))
      .catch(() => {
        // The idle reap of the backend closes the session instead.
      })
  }

  function close(id: string): void {
    const index = tabs.value.findIndex((tab) => tab.id === id)
    const tab = tabs.value[index]
    if (index === -1 || !tab) {
      return
    }
    releaseSession(tab)
    tabs.value = tabs.value.filter((tab) => tab.id !== id)
    if (activeTabId.value === id) {
      const next = tabs.value[Math.max(0, index - 1)]
      activeTabId.value = next ? next.id : null
    }
    // The results of the tab go with the tab, so a closed tab frees its
    // memory.
    useQueryStore().clear(id)
    changed()
  }

  function activate(id: string): void {
    if (tabs.value.some((tab) => tab.id === id)) {
      activeTabId.value = id
      changed()
    }
  }

  function setQuery(id: string, query: string): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab && tab.query !== query) {
      tab.query = query
      tab.dirty = true
      changed()
    }
  }

  function setConnection(id: string, connectionId: string | null): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab) {
      // The session on the old connection belongs to this tab alone, so it
      // goes when the tab moves.
      if (tab.connectionId && tab.connectionId !== connectionId) {
        releaseSession(tab, tab.connectionId)
      }
      tab.connectionId = connectionId
      changed()
    }
  }

  /** Holds the values that the user gave for the parameters of one tab. */
  function setParams(id: string, params: ParamValue[]): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab) {
      tab.params = params
      changed()
    }
  }

  function rename(id: string, title: string): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab && title.trim()) {
      tab.title = title.trim()
      changed()
    }
  }

  function markClean(id: string): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab) {
      tab.dirty = false
      changed()
    }
  }

  /** Builds the record that the workspace file holds. */
  function snapshot(): Workspace {
    return {
      tabs: tabs.value.map((tab) => ({
        id: tab.id,
        title: tab.title,
        query: tab.query,
        connectionId: tab.connectionId,
        dirty: tab.dirty,
        savedQueryId: tab.savedQueryId,
        params: tab.params,
        filePath: tab.filePath,
      })),
      activeTabId: activeTabId.value,
    }
  }

  async function persist(): Promise<void> {
    try {
      await api.saveWorkspace(snapshot())
    } catch {
      // A workspace that cannot be written is not worth an alarm; the tabs
      // stay open for this session.
    }
  }

  /**
   * Compares each tab that names a file with the text of that file. The disk
   * decides the mark: a text that differs carries the mark, and a text that
   * agrees does not. A file that the application cannot read keeps the mark
   * from the workspace record, because the tab then holds the only copy.
   */
  async function reconcileFiles(): Promise<void> {
    await Promise.all(
      tabs.value.map(async (tab) => {
        if (tab.filePath === null) {
          return
        }
        try {
          tab.dirty = (await api.readTextFile(tab.filePath)) !== tab.query
        } catch {
          // The recorded mark stays, because the text on the disk is unknown.
        }
      }),
    )
    changed()
  }

  async function restore(): Promise<void> {
    try {
      const workspace = parseWorkspace(await api.getWorkspace())
      tabs.value = workspace.tabs.map((tab) => ({ ...tab }))
      activeTabId.value = workspace.activeTabId
      // The counter continues after the highest restored title, so a new
      // tab does not repeat the name of a restored one.
      counter = tabs.value.reduce((highest, tab) => {
        const match = /^Query (\d+)$/.exec(tab.title)
        return match ? Math.max(highest, Number(match[1])) : highest
      }, tabs.value.length)
    } catch {
      tabs.value = []
      activeTabId.value = null
      changed()
      return
    }
    await reconcileFiles()
  }

  /** Sets or clears the file that a tab writes back to. */
  function setFilePath(id: string, filePath: string | null): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab) {
      tab.filePath = filePath
      changed()
    }
  }

  /** The tab that already holds one file, when a tab does. */
  function tabForFile(filePath: string): QueryTab | undefined {
    return tabs.value.find((tab) => tab.filePath === filePath)
  }

  return {
    tabs,
    activeTabId,
    revision,
    activeTab,
    hasTabs,
    setFilePath,
    tabForFile,
    add,
    close,
    activate,
    setQuery,
    setConnection,
    setParams,
    rename,
    markClean,
    snapshot,
    persist,
    restore,
  }
})
