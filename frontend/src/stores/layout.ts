import { defineStore } from 'pinia'
import { ref } from 'vue'
import { safeStorage } from './settings'

/** The side panels the rail opens. */
export type Panel = 'connections' | 'explorer' | 'files' | 'history'

/** The two places the results panel can take beside the editor. */
export type ResultsOrientation = 'below' | 'beside'

/** The shape of the work area that a restart brings back. */
export interface Layout {
  /** The side panel that stands open. */
  panel: Panel
  /** True while the side panel takes room beside the work area. */
  panelOpen: boolean
  /** The width of the side panel, in pixels. */
  panelWidth: number
  /** The share of the query view the editor takes, as a percentage. */
  editorSize: number
  /** True while the results panel is a bar below the editor. */
  resultsCollapsed: boolean
  /** The place the results panel takes: below the editor, or beside it. */
  resultsOrientation: ResultsOrientation
}

/** The smallest and the largest share the editor may take. */
export const MIN_EDITOR_SIZE = 15
export const MAX_EDITOR_SIZE = 85

/**
 * The smallest and the largest width of the side panel. The lower figure
 * keeps the filter field and the buttons of a panel header on one row.
 */
export const MIN_PANEL_WIDTH = 220
export const MAX_PANEL_WIDTH = 640

/** The step one arrow key moves the edge of the side panel. */
export const PANEL_WIDTH_STEP = 16

/** The shape a new installation starts with. */
export function defaultLayout(): Layout {
  return {
    panel: 'connections',
    panelOpen: true,
    panelWidth: 320,
    editorSize: 45,
    resultsCollapsed: false,
    resultsOrientation: 'below',
  }
}

/** The key under which the shape of the work area lives in the browser store. */
export const LAYOUT_KEY = 'sql-explorer.layout'

const PANELS: Panel[] = ['connections', 'explorer', 'files', 'history']

const ORIENTATIONS: ResultsOrientation[] = ['below', 'beside']

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
      panelOpen: typeof parsed.panelOpen === 'boolean' ? parsed.panelOpen : defaults.panelOpen,
      panelWidth: clamp(parsed.panelWidth, defaults.panelWidth, MIN_PANEL_WIDTH, MAX_PANEL_WIDTH),
      editorSize: sizeOr(parsed.editorSize, defaults.editorSize),
      resultsCollapsed:
        typeof parsed.resultsCollapsed === 'boolean'
          ? parsed.resultsCollapsed
          : defaults.resultsCollapsed,
      resultsOrientation: ORIENTATIONS.includes(parsed.resultsOrientation as ResultsOrientation)
        ? (parsed.resultsOrientation as ResultsOrientation)
        : defaults.resultsOrientation,
    }
  } catch {
    return defaults
  }
}

/** Keeps a share inside the limits of the split, and falls back for a fault. */
function sizeOr(value: unknown, fallback: number): number {
  return clamp(value, fallback, MIN_EDITOR_SIZE, MAX_EDITOR_SIZE)
}

/** Keeps a figure inside its limits, and falls back when it is not a number. */
function clamp(value: unknown, fallback: number, low: number, high: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return fallback
  }
  return Math.min(high, Math.max(low, Math.round(value)))
}

/**
 * The shape of the work area: the panel that stands open and the place of the
 * split between the editor and the results. The state lives here, and not in
 * the components, because a restart brings it back and because the rail, the
 * commands and the palette all move the panel.
 */
export const useLayoutStore = defineStore('layout', () => {
  const layout = ref<Layout>(defaultLayout())
  /**
   * True while a drag moves the edge of the side panel. A drag gives a new
   * width for every step of the pointer, and a write to the browser store for
   * each step would hold up the frame, so the write waits for the end of the
   * drag. The shell also reads this state, to stop the panel from easing
   * towards each new width while the pointer sets it.
   */
  const resizingPanel = ref(false)

  function load(storage: Pick<Storage, 'getItem'> | null = safeStorage()): void {
    layout.value = parseLayout(storage ? storage.getItem(LAYOUT_KEY) : null)
  }

  function persist(storage: Pick<Storage, 'setItem'> | null = safeStorage()): void {
    storage?.setItem(LAYOUT_KEY, JSON.stringify(layout.value))
  }

  function update(patch: Partial<Layout>): void {
    layout.value = { ...layout.value, ...patch }
    if (!resizingPanel.value) {
      persist()
    }
  }

  /** Opens one panel, whichever panel stood open before. */
  function showPanel(panel: Panel): void {
    update({ panel, panelOpen: true })
  }

  /**
   * Answers a click on the rail. A click on the panel that already stands
   * open closes it, which gives the work area the whole window. A click on
   * any other panel opens that one.
   */
  function selectPanel(panel: Panel): void {
    if (layout.value.panel === panel && layout.value.panelOpen) {
      update({ panelOpen: false })
      return
    }
    showPanel(panel)
  }

  function setPanelOpen(open: boolean): void {
    update({ panelOpen: open })
  }

  function togglePanel(): void {
    setPanelOpen(!layout.value.panelOpen)
  }

  function setPanelWidth(width: number): void {
    update({
      panelWidth: clamp(width, layout.value.panelWidth, MIN_PANEL_WIDTH, MAX_PANEL_WIDTH),
    })
  }

  /** Moves the edge of the side panel by one step, which an arrow key does. */
  function nudgePanelWidth(step: number): void {
    setPanelWidth(layout.value.panelWidth + step)
  }

  function beginPanelResize(): void {
    resizingPanel.value = true
  }

  /** Ends a drag of the panel edge and writes the width the drag left. */
  function endPanelResize(): void {
    if (!resizingPanel.value) {
      return
    }
    resizingPanel.value = false
    persist()
  }

  function setEditorSize(size: number): void {
    update({ editorSize: sizeOr(size, layout.value.editorSize) })
  }

  /**
   * Puts the results panel away, or brings it back. The share of the editor
   * stays as it is, so an expansion gives back the split the user had.
   */
  function setResultsCollapsed(collapsed: boolean): void {
    update({ resultsCollapsed: collapsed })
  }

  function toggleResults(): void {
    setResultsCollapsed(!layout.value.resultsCollapsed)
  }

  /**
   * Moves the results panel between the two places it can take. One share of
   * the editor serves both places.
   */
  function setResultsOrientation(orientation: ResultsOrientation): void {
    update({ resultsOrientation: orientation })
  }

  function toggleResultsOrientation(): void {
    setResultsOrientation(layout.value.resultsOrientation === 'below' ? 'beside' : 'below')
  }

  return {
    layout,
    resizingPanel,
    load,
    persist,
    update,
    showPanel,
    selectPanel,
    setPanelOpen,
    togglePanel,
    setPanelWidth,
    nudgePanelWidth,
    beginPanelResize,
    endPanelResize,
    setEditorSize,
    setResultsCollapsed,
    toggleResults,
    setResultsOrientation,
    toggleResultsOrientation,
  }
})
