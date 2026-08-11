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
      'id' | 'title' | 'query' | 'connectionId' | 'savedQueryId' | 'params' | 'filePath'
    >
  >
  activeTabId: string | null
  /**
   * The folders that the user opened in the last session. The user accepted
   * each one through the dialog of the operating system, and this record
   * holds that acceptance across a restart.
   */
  fileRoots: string[]
}

/** Reads a workspace record and drops anything that is not usable. */
export function parseWorkspace(value: unknown): Workspace {
  const empty: Workspace = { tabs: [], activeTabId: null, fileRoots: [] }
  if (typeof value !== 'object' || value === null) {
    return empty
  }
  const record = value as Record<string, unknown>
  if (!Array.isArray(record.tabs)) {
    return empty
  }
  const fileRoots = Array.isArray(record.fileRoots)
    ? record.fileRoots.filter((root): root is string => typeof root === 'string' && root !== '')
    : []
  const tabs = record.tabs
    .filter((tab): tab is Record<string, unknown> => typeof tab === 'object' && tab !== null)
    .filter((tab) => typeof tab.id === 'string' && typeof tab.query === 'string')
    .map((tab) => ({
      id: tab.id as string,
      title: typeof tab.title === 'string' ? tab.title : 'Query',
      query: tab.query as string,
      connectionId: typeof tab.connectionId === 'string' ? tab.connectionId : null,
      savedQueryId: typeof tab.savedQueryId === 'string' ? tab.savedQueryId : null,
      params: parseParamValues(tab.params),
      filePath: typeof tab.filePath === 'string' && tab.filePath !== '' ? tab.filePath : null,
    }))
  const activeTabId =
    typeof record.activeTabId === 'string' && tabs.some((tab) => tab.id === record.activeTabId)
      ? record.activeTabId
      : (tabs[0]?.id ?? null)
  return { tabs, activeTabId, fileRoots }
}

export const useTabsStore = defineStore('tabs', () => {
  const connections = useConnectionsStore()

  const tabs = ref<QueryTab[]>([])
  const activeTabId = ref<string | null>(null)
  /** The folders of the files panel, which the workspace file holds. */
  const fileRoots = ref<string[]>([])
  let counter = 0

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
  }

  function closeOthers(id: string): void {
    const queries = useQueryStore()
    for (const tab of tabs.value) {
      if (tab.id !== id) {
        releaseSession(tab)
        queries.clear(tab.id)
      }
    }
    tabs.value = tabs.value.filter((tab) => tab.id === id)
    activeTabId.value = tabs.value[0]?.id ?? null
  }

  function closeAll(): void {
    const queries = useQueryStore()
    for (const tab of tabs.value) {
      releaseSession(tab)
      queries.clear(tab.id)
    }
    tabs.value = []
    activeTabId.value = null
  }

  function activate(id: string): void {
    if (tabs.value.some((tab) => tab.id === id)) {
      activeTabId.value = id
    }
  }

  function setQuery(id: string, query: string): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab && tab.query !== query) {
      tab.query = query
      tab.dirty = true
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
    }
  }

  /** Holds the values that the user gave for the parameters of one tab. */
  function setParams(id: string, params: ParamValue[]): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab) {
      tab.params = params
    }
  }

  function rename(id: string, title: string): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab && title.trim()) {
      tab.title = title.trim()
    }
  }

  function markClean(id: string): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab) {
      tab.dirty = false
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
        savedQueryId: tab.savedQueryId,
        params: tab.params,
        filePath: tab.filePath,
      })),
      activeTabId: activeTabId.value,
      fileRoots: fileRoots.value,
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

  async function restore(): Promise<void> {
    try {
      const workspace = parseWorkspace(await api.getWorkspace())
      tabs.value = workspace.tabs.map((tab) => ({ ...tab, dirty: false }))
      activeTabId.value = workspace.activeTabId
      await restoreFileRoots(workspace.fileRoots)
      // The counter continues after the highest restored title, so a new
      // tab does not repeat the name of a restored one.
      counter = tabs.value.reduce((highest, tab) => {
        const match = /^Query (\d+)$/.exec(tab.title)
        return match ? Math.max(highest, Number(match[1])) : highest
      }, tabs.value.length)
    } catch {
      tabs.value = []
      activeTabId.value = null
      fileRoots.value = []
    }
  }

  /**
   * Gives the folders of the last session back to the backend, which guards
   * every read and every write against them. A folder that is gone from the
   * disk, or that the backend refuses, is dropped from the record.
   */
  async function restoreFileRoots(roots: string[]): Promise<void> {
    const kept: string[] = []
    for (const root of roots) {
      try {
        if (await api.restoreFolder(root)) {
          kept.push(root)
        }
      } catch {
        // A folder that the backend cannot take is left out of the record.
      }
    }
    fileRoots.value = kept
  }

  /** Records a folder that the user opened in this session. */
  function addFileRoot(root: string): void {
    if (!fileRoots.value.includes(root)) {
      fileRoots.value = [...fileRoots.value, root]
    }
  }

  /** Takes a folder out of the panel of this session and of the record. */
  function removeFileRoot(root: string): void {
    fileRoots.value = fileRoots.value.filter((held) => held !== root)
  }

  /** Sets or clears the file that a tab writes back to. */
  function setFilePath(id: string, filePath: string | null): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab) {
      tab.filePath = filePath
    }
  }

  /** The tab that already holds one file, when a tab does. */
  function tabForFile(filePath: string): QueryTab | undefined {
    return tabs.value.find((tab) => tab.filePath === filePath)
  }

  return {
    tabs,
    activeTabId,
    activeTab,
    hasTabs,
    fileRoots,
    addFileRoot,
    removeFileRoot,
    setFilePath,
    tabForFile,
    add,
    close,
    closeOthers,
    closeAll,
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
