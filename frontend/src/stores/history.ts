import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api } from '@/lib/api'
import { createId } from './connections'
import { useUiStore } from './ui'
import type { HistoryEntry } from '@/types/api'

/**
 * The number of history entries the list keeps. The backend file keeps the
 * same number, so the two lists hold the same entries.
 */
export const HISTORY_LIMIT = 500

/**
 * The most text that the statements and the error texts of the list hold
 * together, in UTF-16 units. The backend file keeps the same budget, so 500
 * runs of a large script do not make a file of hundreds of megabytes.
 */
export const HISTORY_TEXT_BUDGET = 4 * 1024 * 1024

/**
 * The entries of a list that stay under the limit and under the budget of the
 * text, newest first. The newest entry stays also when its text alone passes
 * the budget.
 */
export function trimHistory(entries: HistoryEntry[]): HistoryEntry[] {
  let total = 0
  const kept = entries.slice(0, HISTORY_LIMIT)
  const over = kept.findIndex((entry) => {
    total += entry.query.length + (entry.error?.length ?? 0)
    return total > HISTORY_TEXT_BUDGET
  })
  return over === -1 ? kept : kept.slice(0, Math.max(1, over))
}

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

  async function load(): Promise<void> {
    loading.value = true
    try {
      entries.value = await api.getHistory()
    } catch (error) {
      ui.reportError(error)
    } finally {
      loading.value = false
    }
  }

  /**
   * Puts one entry at the front of the list and drops the entries above the
   * limit and the budget of the text. An entry that repeats the statement at
   * the front replaces it. The backend file follows the same rules.
   */
  function putEntry(entry: HistoryEntry): void {
    const first = entries.value[0]
    const rest =
      first && first.query === entry.query && first.connectionId === entry.connectionId
        ? entries.value.slice(1)
        : entries.value
    entries.value = trimHistory([entry, ...rest])
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
      ui.success('History cleared.')
    } catch (error) {
      ui.reportError(error)
    }
  }

  return {
    entries,
    filter,
    loading,
    visibleEntries,
    load,
    record,
    clear,
  }
})
