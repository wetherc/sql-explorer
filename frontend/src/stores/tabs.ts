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
}

/** The shape the open tabs take in the workspace file. */
export interface Workspace {
  tabs: Array<Pick<QueryTab, 'id' | 'title' | 'query' | 'connectionId' | 'savedQueryId' | 'params'>>
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
      savedQueryId: typeof tab.savedQueryId === 'string' ? tab.savedQueryId : null,
      params: parseParamValues(tab.params),
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

  const activeTab = computed(() => tabs.value.find((tab) => tab.id === activeTabId.value) ?? null)
  const hasTabs = computed(() => tabs.value.length > 0)

  function nextTitle(): string {
    counter += 1
    return `Query ${counter}`
  }

  function add(
    options: { connectionId?: string | null; query?: string; title?: string } = {},
  ): QueryTab {
    const tab: QueryTab = {
      id: createId(),
      title: options.title ?? nextTitle(),
      query: options.query ?? '',
      connectionId: options.connectionId ?? connections.selectedId,
      dirty: false,
      savedQueryId: null,
      params: [],
    }
    tabs.value = [...tabs.value, tab]
    activeTabId.value = tab.id
    return tab
  }

  function close(id: string): void {
    const index = tabs.value.findIndex((tab) => tab.id === id)
    if (index === -1) {
      return
    }
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
        queries.clear(tab.id)
      }
    }
    tabs.value = tabs.value.filter((tab) => tab.id === id)
    activeTabId.value = tabs.value[0]?.id ?? null
  }

  function closeAll(): void {
    const queries = useQueryStore()
    for (const tab of tabs.value) {
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

  async function restore(): Promise<void> {
    try {
      const workspace = parseWorkspace(await api.getWorkspace())
      tabs.value = workspace.tabs.map((tab) => ({ ...tab, dirty: false }))
      activeTabId.value = workspace.activeTabId
      counter = tabs.value.length
    } catch {
      tabs.value = []
      activeTabId.value = null
    }
  }

  return {
    tabs,
    activeTabId,
    activeTab,
    hasTabs,
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
