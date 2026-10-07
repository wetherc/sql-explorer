import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api } from '@/lib/api'
import { useTabsStore } from './tabs'
import { useUiStore } from './ui'
import type { FolderEntry } from '@/types/api'

/** One node of the files tree. */
export interface FileNode {
  /** The path of the entry, which is also its key in the tree. */
  path: string
  name: string
  entryType: 'folder' | 'file'
  /** The depth of the node, so the tree can indent its rows. */
  depth: number
  /** Missing on a file, which never holds children. */
  children?: FileNode[]
  loading: boolean
  loaded: boolean
  /** Why the last read of a folder failed, or null after a read that did not fail. */
  error: string | null
}

/** Builds a node from one entry of a folder. */
export function nodeOfEntry(entry: FolderEntry, depth: number): FileNode {
  return {
    path: entry.path,
    name: entry.name,
    entryType: entry.entryType,
    depth,
    children: entry.entryType === 'folder' ? [] : undefined,
    loading: false,
    loaded: false,
    error: null,
  }
}

/** The name of a file, without the folders in front of it. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

/** Finds one node by its path, wherever it stands in the tree. */
export function findNode(nodes: FileNode[], path: string): FileNode | undefined {
  for (const node of nodes) {
    if (node.path === path) {
      return node
    }
    const found = node.children ? findNode(node.children, path) : undefined
    if (found) {
      return found
    }
  }
  return undefined
}

/**
 * The rows the panel draws: each root, and the children of every folder that
 * stands open.
 */
export function visibleRows(nodes: FileNode[], openPaths: Set<string>): FileNode[] {
  const rows: FileNode[] = []
  for (const node of nodes) {
    rows.push(node)
    if (node.entryType === 'folder' && openPaths.has(node.path) && node.children) {
      rows.push(...visibleRows(node.children, openPaths))
    }
  }
  return rows
}

/**
 * The folders of the files panel and the entries the panel has read.
 *
 * The backend guards every path against the folders that the user accepted,
 * so this store holds no rule of its own about what is reachable. It holds
 * one level of each folder and reads the next level as the user opens it, so
 * a folder with very many entries costs nothing until it is opened.
 */
export const useFilesStore = defineStore('files', () => {
  const tabs = useTabsStore()
  const ui = useUiStore()

  /** One node for each folder the user opened. */
  const roots = ref<FileNode[]>([])
  const openPaths = ref<Set<string>>(new Set())
  const loading = ref(false)

  const rows = computed(() => visibleRows(roots.value, openPaths.value))
  const hasRoots = computed(() => roots.value.length > 0)

  /** Asks the user for a folder and adds it to the panel. */
  async function openFolder(): Promise<void> {
    loading.value = true
    try {
      const path = await api.pickFolder()
      if (path) {
        addRoot(path)
        await expand(path)
      }
    } catch (error) {
      ui.reportError(error)
    } finally {
      loading.value = false
    }
  }

  /**
   * Puts the folders that the backend records back into the panel. The
   * backend owns that record, so the panel shows the folders the user
   * accepted and nothing else.
   */
  async function restoreRoots(): Promise<void> {
    roots.value = []
    openPaths.value = new Set()
    try {
      for (const path of await api.fileRoots()) {
        addRoot(path)
      }
    } catch (error) {
      ui.reportError(error)
    }
  }

  /** Adds one folder as a root, unless the panel already holds it. */
  function addRoot(path: string): void {
    if (roots.value.some((root) => root.path === path)) {
      return
    }
    roots.value = [
      ...roots.value,
      {
        path,
        name: baseName(path),
        entryType: 'folder',
        depth: 0,
        children: [],
        loading: false,
        loaded: false,
        error: null,
      },
    ]
  }

  /**
   * Takes one folder out of the panel and out of the record of the backend.
   * A file under the folder is no longer reachable after the call, so a tab
   * that holds such a file cannot write it back.
   */
  async function closeRoot(path: string): Promise<void> {
    roots.value = roots.value.filter((root) => root.path !== path)
    const open = new Set(openPaths.value)
    open.delete(path)
    openPaths.value = open
    try {
      await api.closeFolder(path)
    } catch (error) {
      ui.reportError(error)
    }
  }

  /**
   * The number of the last read of each folder. A refresh starts a read
   * while an older one runs, and the answer of the older read is dropped.
   */
  const loadGeneration = new Map<string, number>()

  /**
   * Reads the entries of one folder and writes them into it. The entries
   * are new nodes, so each folder below that the panel shows open is read
   * too, and it does not stand open and empty.
   */
  async function readFolder(node: FileNode): Promise<void> {
    const generation = (loadGeneration.get(node.path) ?? 0) + 1
    loadGeneration.set(node.path, generation)
    const isLast = () => loadGeneration.get(node.path) === generation
    node.loading = true
    node.error = null
    let children: FileNode[]
    try {
      const entries = await api.listFolder(node.path)
      if (!isLast()) {
        return
      }
      children = entries.map((entry) => nodeOfEntry(entry, node.depth + 1))
      node.children = children
      node.loaded = true
    } catch (error) {
      if (isLast()) {
        // The panel shows the failure in the folder with a way to read it
        // again, so the notice in the corner leaves on its own.
        node.error = ui.reportError(error, { kept: true }).message
        node.children = []
        node.loaded = false
      }
      return
    } finally {
      if (isLast()) {
        node.loading = false
      }
    }
    for (const child of children) {
      if (child.entryType === 'folder' && openPaths.value.has(child.path)) {
        await readFolder(child)
      }
    }
  }

  /**
   * Opens one folder and reads its entries. A folder that was read before
   * is read again, because files can come and go on the disk while the
   * folder stands shut. A read that already runs is not started twice.
   */
  async function expand(path: string): Promise<void> {
    const node = findNode(roots.value, path)
    if (!node || node.entryType !== 'folder') {
      return
    }
    openPaths.value = new Set(openPaths.value).add(path)
    if (!node.loading) {
      await readFolder(node)
    }
  }

  function collapse(path: string): void {
    const open = new Set(openPaths.value)
    open.delete(path)
    openPaths.value = open
  }

  /**
   * Reads the entries of one folder again, whether they were read or not,
   * and also while a read of the folder runs.
   */
  async function refresh(path: string): Promise<void> {
    const node = findNode(roots.value, path)
    if (!node || node.entryType !== 'folder') {
      return
    }
    await readFolder(node)
  }

  /**
   * Opens a file in a tab. A file that a tab already holds brings that tab
   * forward instead of opening a second one.
   */
  async function openFile(path: string): Promise<void> {
    const held = tabs.tabForFile(path)
    if (held) {
      tabs.activate(held.id)
      return
    }
    // A second click during the read opens no second tab.
    if (opening.has(path)) {
      return
    }
    opening.add(path)
    try {
      const { contents, encoding } = await api.readTextFile(path)
      // Another way to open the file can have opened it during the read.
      const opened = tabs.tabForFile(path)
      if (opened) {
        tabs.activate(opened.id)
        return
      }
      tabs.add({ query: contents, title: baseName(path), filePath: path, encoding })
    } catch (error) {
      ui.reportError(error)
    } finally {
      opening.delete(path)
    }
  }

  /** The paths of the files whose read for a new tab runs. */
  const opening = new Set<string>()

  /**
   * Asks the user for one statement file and opens it in a tab. The backend
   * admits that one file, so a later save of the tab reaches it. The folder
   * of the file stays out of the panel, because the user accepted the file
   * alone.
   */
  async function openFileFromDialog(): Promise<void> {
    loading.value = true
    try {
      const opened = await api.openStatementFile()
      if (!opened) {
        return
      }
      const held = tabs.tabForFile(opened.path)
      if (held) {
        tabs.activate(held.id)
        return
      }
      tabs.add({
        query: opened.contents,
        title: baseName(opened.path),
        filePath: opened.path,
        encoding: opened.encoding,
      })
    } catch (error) {
      ui.reportError(error)
    } finally {
      loading.value = false
    }
  }

  return {
    roots,
    openPaths,
    loading,
    rows,
    hasRoots,
    openFolder,
    openFileFromDialog,
    restoreRoots,
    closeRoot,
    expand,
    collapse,
    refresh,
    openFile,
  }
})
