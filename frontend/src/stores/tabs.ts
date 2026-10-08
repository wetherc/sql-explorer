import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api } from '@/lib/api'
import { fullErrorText, toErrorPayload } from '@/lib/errors'
import { parseParamValues } from '@/lib/params'
import { TEXT_ENCODINGS, type ParamValue, type TextEncoding } from '@/types/api'
import { useConnectionsStore } from './connections'
import { createId } from './connections'
import { useQueryStore } from './query'
import { useUiStore } from './ui'

export interface QueryTab {
  id: string
  title: string
  query: string
  connectionId: string | null
  /** True when the text differs from the text the tab last saved or read. */
  dirty: boolean
  /** The values the user gave for the named parameters of the statement. */
  params: ParamValue[]
  /** The file on the disk this tab came from, when it came from one. */
  filePath: string | null
  /** The encoding of that file, so a save writes the file as it was. A tab
   *  without one writes UTF-8. */
  encoding?: TextEncoding
}

/** The open tabs as the workspace file keeps them. */
export interface Workspace {
  tabs: QueryTab[]
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
      params: parseParamValues(tab.params),
      filePath: typeof tab.filePath === 'string' && tab.filePath !== '' ? tab.filePath : null,
      encoding: TEXT_ENCODINGS.includes(tab.encoding as TextEncoding)
        ? (tab.encoding as TextEncoding)
        : 'utf8',
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
  /**
   * The text that each tab last saved or read, by the identifier of the tab.
   * A tab whose text comes back to this text loses its mark, so an undo of
   * each change leaves the tab clean. A tab with no entry here stays marked
   * after each change, because the text it came from is unknown.
   */
  const cleanText = new Map<string, string>()
  /** True after a write of the workspace failed, until one succeeds, so the
   *  failure gives one notice and not one for each change. */
  let persistFailed = false
  /** True when the workspace of the last session could not be read. The
   *  file then stays as it is until the user opens a tab, so a restart
   *  can try it again. */
  let restoreFailed = false
  /** Settles when restore() has put the tabs of the last session in the
   *  list, or null when no restore runs. A write before that moment would
   *  replace the tabs of the last session with the tabs that opened during
   *  the read, so a write waits for it. */
  let restoring: Promise<void> | null = null

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
      encoding?: TextEncoding
    } = {},
  ): QueryTab {
    const tab: QueryTab = {
      id: createId(),
      title: options.title ?? nextTitle(),
      query: options.query ?? '',
      connectionId: options.connectionId ?? connections.selectedId,
      dirty: false,
      params: [],
      filePath: options.filePath ?? null,
      encoding: options.encoding ?? 'utf8',
    }
    cleanText.set(tab.id, tab.query)
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
    // The release rolls back the transaction of the session.
    queries.forgetTransaction(tab.id)
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
    cleanText.delete(id)
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
      tab.dirty = cleanText.get(id) !== query
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

  /**
   * Records the text that a save wrote. A save awaits the disk or the
   * library, and the user can type while it runs, so the caller gives the
   * text that went out. A tab whose text changed since then keeps its mark.
   */
  function markClean(id: string, text: string): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab) {
      cleanText.set(id, text)
      tab.dirty = tab.query !== text
      changed()
    }
  }

  /** Records the encoding that the file of a tab has on the disk. */
  function setEncoding(id: string, encoding: TextEncoding): void {
    const tab = tabs.value.find((item) => item.id === id)
    if (tab) {
      tab.encoding = encoding
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
        params: tab.params,
        filePath: tab.filePath,
        encoding: tab.encoding,
      })),
      activeTabId: activeTabId.value,
    }
  }

  // The last write of the tabs in the queue. Each write starts after the one
  // before it ends. Two writes at once reach the backend on two threads, and
  // the older record could then reach the disk last.
  let lastWrite: Promise<void> = Promise.resolve()

  /**
   * Writes the tabs to the workspace file after every earlier write ends. The
   * record is built when the write starts, so a write that waited writes the
   * newest tabs.
   */
  function persist(): Promise<void> {
    lastWrite = lastWrite.then(writeWorkspace)
    return lastWrite
  }

  /** Settles when every write that is in the queue has ended. */
  function settled(): Promise<void> {
    return lastWrite
  }

  async function writeWorkspace(): Promise<void> {
    // A write that the close of the window asks for during the read waits
    // too, so a tab that opened during the read reaches the file.
    if (restoring) {
      await restoring
    }
    if (restoreFailed && tabs.value.length === 0) {
      return
    }
    try {
      await api.saveWorkspace(snapshot())
      persistFailed = false
      restoreFailed = false
    } catch (error) {
      if (!persistFailed) {
        persistFailed = true
        const payload = toErrorPayload(error)
        useUiStore().reportError({
          ...payload,
          message: "Couldn't save the open tabs. They may not open again after a restart.",
          detail: fullErrorText(payload),
        })
      }
    }
  }

  /**
   * Compares each tab that names a file with the text of that file. The disk
   * decides the mark: a text that differs carries the mark, and a text that
   * agrees does not. A file that the application cannot read keeps the mark
   * from the workspace record, because the tab then holds the only copy.
   */
  async function reconcileFiles(list: QueryTab[]): Promise<void> {
    await Promise.all(
      list.map(async (tab) => {
        if (tab.filePath === null) {
          return
        }
        try {
          const { contents: text, encoding } = await api.readTextFile(tab.filePath)
          tab.encoding = encoding
          cleanText.set(tab.id, text)
          tab.dirty = text !== tab.query
        } catch {
          // The recorded mark stays, because the text on the disk is unknown.
        }
      }),
    )
    changed()
  }

  /**
   * Puts the tabs of the last session back.
   *
   * The menu of the system, a key or a button can open a tab before the read
   * of the workspace file ends. The restored tabs therefore go in front of the
   * tabs that are open, and do not replace them. This covers every path that
   * opens a tab, and no command has to wait for the read. A tab that the user
   * opened stays the active tab.
   *
   * The read starts after `before` settles, and a write of the workspace
   * waits from the call on. The caller gives the work that must end before
   * the read, so no write can slip in between.
   */
  async function restore(before: Promise<void> = Promise.resolve()): Promise<void> {
    let done!: () => void
    restoring = new Promise((resolve) => {
      done = resolve
    })
    let restored: QueryTab[] | null = null
    try {
      await before
      restored = await restoreTabs()
    } finally {
      restoring = null
      done()
    }
    if (restored) {
      // The tabs of the store are reactive, so the compare changes them and
      // not the plain records of the file.
      await reconcileFiles(restored)
    }
  }

  /**
   * Reads the workspace file and merges its tabs in front of the open tabs.
   * Gives back the restored tabs as the store keeps them, or null when the
   * file could not be read. The caller compares their files with the disk
   * after the merge, so a write that waits for the restore does not wait
   * for the disk as well.
   */
  async function restoreTabs(): Promise<QueryTab[] | null> {
    let workspace: Workspace
    try {
      workspace = parseWorkspace(await api.getWorkspace())
    } catch (error) {
      restoreFailed = true
      const payload = toErrorPayload(error)
      useUiStore().reportError({
        ...payload,
        message: "Couldn't open the tabs of the last session.",
        detail: fullErrorText(payload),
      })
      changed()
      return null
    }
    const restored: QueryTab[] = workspace.tabs.map((tab) => ({ ...tab }))
    const replaced = mergeSameFiles(restored, tabs.value)
    const opened = tabs.value.filter((tab) => !restored.includes(tab))
    // The workspace file holds no copy of the saved text, so a tab that
    // carries the mark stays marked until the next save.
    for (const tab of restored) {
      if (!tab.dirty) {
        cleanText.set(tab.id, tab.query)
      }
    }
    tabs.value = [...restored, ...opened]
    const lastActive = workspace.activeTabId
    activeTabId.value =
      activeTabId.value ?? (lastActive ? (replaced.get(lastActive) ?? lastActive) : null)
    // The counter continues after the highest restored title, so a new
    // tab does not repeat the name of a restored one.
    counter = restored.reduce(
      (highest, tab) => {
        const match = /^Query (\d+)$/.exec(tab.title)
        return match ? Math.max(highest, Number(match[1])) : highest
      },
      Math.max(counter, restored.length),
    )
    // A tab that opened during the read took the next free number of that
    // moment, which a restored tab can have as well.
    const restoredTitles = new Set(restored.map((tab) => tab.title))
    for (const tab of opened) {
      if (/^Query \d+$/.test(tab.title) && restoredTitles.has(tab.title)) {
        tab.title = nextTitle()
      }
    }
    changed()
    return tabs.value.slice(0, restored.length)
  }

  /**
   * Makes one tab of each file that a restored tab and a tab that opened
   * during the read both name. The tab that opened stays, because a run or
   * an edit of the user can already belong to it, and it takes the place
   * of the restored tab in the list. Gives back the identifier of the tab
   * that took each place.
   *
   * The text of the merged tab follows the edits of the user:
   *
   * - When the opened tab has no edit, it takes the text, the mark, the
   *   title and the parameters of the restored tab. Unsaved edits of the
   *   last session then stay. It takes the connection as well, unless it
   *   ran a statement, because a run opened a session on its own
   *   connection.
   * - When the opened tab has an edit and the restored tab has none, the
   *   opened tab keeps its text.
   * - When both have edits, both tabs stay. The application cannot merge
   *   two edits of one file, and one tab would lose the edits of the other.
   *
   * The compare is on the path text that the backend gave each tab. The
   * backend gives the path that the dialog or the folder list gave it, so
   * two tabs of one file match when the user reached the file the same way.
   */
  function mergeSameFiles(restored: QueryTab[], open: QueryTab[]): Map<string, string> {
    const replaced = new Map<string, string>()
    const queries = useQueryStore()
    for (const tab of open) {
      const index = restored.findIndex(
        (item) => tab.filePath !== null && item.filePath === tab.filePath,
      )
      const old = restored[index]
      if (!old || (tab.dirty && old.dirty)) {
        continue
      }
      if (!tab.dirty) {
        tab.query = old.query
        tab.dirty = old.dirty
        tab.title = old.title
        tab.params = old.params
        if ((queries.peekState(tab.id)?.lastRunAt ?? null) === null) {
          tab.connectionId = old.connectionId
        }
      }
      restored[index] = tab
      replaced.set(old.id, tab.id)
    }
    return replaced
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
    setEncoding,
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
    settled,
  }
})
