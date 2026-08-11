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
  kind: 'folder' | 'file'
  /** The depth of the node, so the tree can indent its rows. */
  depth: number
  /** Missing on a file, which never holds children. */
  children?: FileNode[]
  loading: boolean
  loaded: boolean
}

/** Builds a node from one entry of a folder. */
export function nodeOfEntry(entry: FolderEntry, depth: number): FileNode {
  return {
    path: entry.path,
    name: entry.name,
    kind: entry.kind,
    depth,
    children: entry.kind === 'folder' ? [] : undefined,
    loading: false,
    loaded: false,
  }
}

/** The name of a file, without the folders in front of it. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

/**
 * The folder that holds a file, or `null` when the path names no folder in
 * front of the file. The two marks of a path are both read, because the
 * backend gives the path in the form of the operating system.
 */
export function folderOf(path: string): string | null {
  const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  if (cut <= 0) {
    return null
  }
  return path.slice(0, cut)
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
    if (node.kind === 'folder' && openPaths.has(node.path) && node.children) {
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
        tabs.addFileRoot(path)
        await expand(path)
      }
    } catch (error) {
      ui.reportError(error)
    } finally {
      loading.value = false
    }
  }

  /** Puts the folders of the workspace back into the panel. */
  function restoreRoots(paths: string[]): void {
    roots.value = []
    openPaths.value = new Set()
    for (const path of paths) {
      addRoot(path)
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
        kind: 'folder',
        depth: 0,
        children: [],
        loading: false,
        loaded: false,
      },
    ]
  }

  /** Takes one folder out of the panel and out of the workspace. */
  function closeRoot(path: string): void {
    roots.value = roots.value.filter((root) => root.path !== path)
    const open = new Set(openPaths.value)
    open.delete(path)
    openPaths.value = open
    tabs.removeFileRoot(path)
  }

  /** Reads the entries of one folder, unless they are already read. */
  async function loadFolder(node: FileNode): Promise<void> {
    if (node.loaded || node.loading) {
      return
    }
    node.loading = true
    try {
      const entries = await api.listFolder(node.path)
      node.children = entries.map((entry) => nodeOfEntry(entry, node.depth + 1))
      node.loaded = true
    } catch (error) {
      ui.reportError(error)
    } finally {
      node.loading = false
    }
  }

  /** Opens one folder and reads its entries. */
  async function expand(path: string): Promise<void> {
    const node = findNode(roots.value, path)
    if (!node || node.kind !== 'folder') {
      return
    }
    openPaths.value = new Set(openPaths.value).add(path)
    await loadFolder(node)
  }

  function collapse(path: string): void {
    const open = new Set(openPaths.value)
    open.delete(path)
    openPaths.value = open
  }

  /** Reads the entries of one folder again, whether they were read or not. */
  async function refresh(path: string): Promise<void> {
    const node = findNode(roots.value, path)
    if (!node || node.kind !== 'folder') {
      return
    }
    node.loaded = false
    await loadFolder(node)
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
    try {
      const text = await api.readTextFile(path)
      tabs.add({ query: text, title: baseName(path), filePath: path })
    } catch (error) {
      ui.reportError(error)
    }
  }

  /**
   * Asks the user for one statement file and opens it in a tab. The folder
   * of the file joins the panel, so the work beside that file is one click
   * away and a later save of the tab reaches the file.
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
      })
      const folder = folderOf(opened.path)
      if (folder !== null) {
        addRoot(folder)
        tabs.addFileRoot(folder)
        await expand(folder)
      }
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
