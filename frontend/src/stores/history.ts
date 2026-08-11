import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api } from '@/lib/api'
import { createId } from './connections'
import { useUiStore } from './ui'
import type { HistoryEntry, SavedQuery } from '@/types/api'

/**
 * The number of history entries the list keeps. The backend file keeps the
 * same number, so the two lists hold the same entries.
 */
export const HISTORY_LIMIT = 500

/** What the query store reports after one execution. */
export interface HistoryInput {
  connectionId: string
  connectionName: string
  query: string
  elapsedMs: number
  rowCount: number
  succeeded: boolean
  error: string | null
}

export const useHistoryStore = defineStore('history', () => {
  const ui = useUiStore()

  const entries = ref<HistoryEntry[]>([])
  const savedQueries = ref<SavedQuery[]>([])
  const filter = ref('')
  const loading = ref(false)

  const visibleEntries = computed(() => {
    const needle = filter.value.trim().toLowerCase()
    if (needle === '') {
      return entries.value
    }
    return entries.value.filter(
      (entry) =>
        entry.query.toLowerCase().includes(needle) ||
        entry.connectionName.toLowerCase().includes(needle),
    )
  })

  const visibleSavedQueries = computed(() => {
    const needle = filter.value.trim().toLowerCase()
    if (needle === '') {
      return savedQueries.value
    }
    return savedQueries.value.filter(
      (query) =>
        query.name.toLowerCase().includes(needle) || query.query.toLowerCase().includes(needle),
    )
  })

  /** The folders that hold the saved statements. */
  const folders = computed(() => {
    const names = new Set<string>()
    for (const query of savedQueries.value) {
      names.add(query.folder?.trim() || 'Saved queries')
    }
    return [...names].sort((left, right) => left.localeCompare(right))
  })

  async function load(): Promise<void> {
    loading.value = true
    try {
      entries.value = await api.getHistory()
      savedQueries.value = await api.getSavedQueries()
    } catch (error) {
      ui.reportError(error)
    } finally {
      loading.value = false
    }
  }

  /**
   * Puts one entry at the front of the list and drops the entries above the
   * limit. An entry that repeats the statement at the front replaces it. The
   * backend file follows the same two rules.
   */
  function putEntry(entry: HistoryEntry): void {
    const first = entries.value[0]
    const rest =
      first && first.query === entry.query && first.connectionId === entry.connectionId
        ? entries.value.slice(1)
        : entries.value
    entries.value = [entry, ...rest].slice(0, HISTORY_LIMIT)
  }

  /** Adds one execution to the history. */
  async function record(input: HistoryInput): Promise<void> {
    const entry: HistoryEntry = {
      id: createId(),
      connectionId: input.connectionId,
      connectionName: input.connectionName,
      query: input.query,
      ranAt: new Date().toISOString(),
      elapsedMs: input.elapsedMs,
      rowCount: input.rowCount,
      succeeded: input.succeeded,
      error: input.error,
    }
    putEntry(entry)
    try {
      await api.addHistoryEntry(entry)
    } catch {
      // The history is a convenience. A failure to write it must not stop
      // the result of the statement from reaching the user, so the entry
      // is kept for this session only.
    }
  }

  async function clear(): Promise<void> {
    try {
      await api.clearHistory()
      entries.value = []
      ui.success('The history is empty.')
    } catch (error) {
      ui.reportError(error)
    }
  }

  async function save(query: {
    id?: string
    name: string
    query: string
    connectionId?: string | null
    folder?: string | null
  }): Promise<SavedQuery | null> {
    if (!query.name.trim()) {
      ui.warn('A saved statement needs a name.')
      return null
    }
    const record: SavedQuery = {
      id: query.id ?? createId(),
      name: query.name.trim(),
      query: query.query,
      connectionId: query.connectionId ?? null,
      folder: query.folder?.trim() || null,
      updatedAt: new Date().toISOString(),
    }
    try {
      await api.saveQuery(record)
      savedQueries.value = await api.getSavedQueries()
      ui.success(`The statement '${record.name}' is saved.`)
      return record
    } catch (error) {
      ui.reportError(error)
      return null
    }
  }

  async function remove(id: string): Promise<void> {
    try {
      await api.deleteSavedQuery(id)
      savedQueries.value = savedQueries.value.filter((query) => query.id !== id)
    } catch (error) {
      ui.reportError(error)
    }
  }

  return {
    entries,
    savedQueries,
    filter,
    loading,
    visibleEntries,
    visibleSavedQueries,
    folders,
    load,
    record,
    clear,
    save,
    remove,
  }
})
