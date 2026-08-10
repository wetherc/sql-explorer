import { defineStore } from 'pinia'
import { ref } from 'vue'
import { safeStorage } from './settings'

/** The three side panels the rail opens. */
export type Panel = 'connections' | 'explorer' | 'history'

/** The shape of the work area that a restart brings back. */
export interface Layout {
  /** The side panel that stands open. */
  panel: Panel
  /** The share of the query view the editor takes, as a percentage. */
  editorSize: number
}

/** The smallest and the largest share the editor may take. */
export const MIN_EDITOR_SIZE = 15
export const MAX_EDITOR_SIZE = 85

/** The shape a new installation starts with. */
export function defaultLayout(): Layout {
  return { panel: 'connections', editorSize: 45 }
}

/** The key under which the shape of the work area lives in the browser store. */
export const LAYOUT_KEY = 'sql-explorer.layout'

const PANELS: Panel[] = ['connections', 'explorer', 'history']

/** Reads the shape and falls back on the defaults for a bad record. */
export function parseLayout(raw: string | null): Layout {
  const defaults = defaultLayout()
  if (!raw) {
    return defaults
  }
  try {
    const parsed = JSON.parse(raw) as Partial<Layout>
    return {
      panel: PANELS.includes(parsed.panel as Panel) ? (parsed.panel as Panel) : defaults.panel,
      editorSize: sizeOr(parsed.editorSize, defaults.editorSize),
    }
  } catch {
    return defaults
  }
}

/** Keeps a share inside the limits of the split, and falls back for a fault. */
function sizeOr(value: unknown, fallback: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return fallback
  }
  return Math.min(MAX_EDITOR_SIZE, Math.max(MIN_EDITOR_SIZE, Math.round(value)))
}

/**
 * The shape of the work area: the panel that stands open and the place of the
 * split between the editor and the results. The state lives here, and not in
 * the components, because a restart brings it back and because the rail, the
 * commands and the palette all move the panel.
 */
export const useLayoutStore = defineStore('layout', () => {
  const layout = ref<Layout>(defaultLayout())

  function load(storage: Pick<Storage, 'getItem'> | null = safeStorage()): void {
    layout.value = parseLayout(storage ? storage.getItem(LAYOUT_KEY) : null)
  }

  function persist(storage: Pick<Storage, 'setItem'> | null = safeStorage()): void {
    storage?.setItem(LAYOUT_KEY, JSON.stringify(layout.value))
  }

  function update(patch: Partial<Layout>): void {
    layout.value = { ...layout.value, ...patch }
    persist()
  }

  function showPanel(panel: Panel): void {
    update({ panel })
  }

  function setEditorSize(size: number): void {
    update({ editorSize: sizeOr(size, layout.value.editorSize) })
  }

  return { layout, load, persist, update, showPanel, setEditorSize }
})
