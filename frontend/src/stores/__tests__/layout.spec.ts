import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import {
  LAYOUT_KEY,
  MAX_EDITOR_SIZE,
  MAX_PANEL_WIDTH,
  MIN_EDITOR_SIZE,
  MIN_PANEL_WIDTH,
  PANEL_WIDTH_STEP,
  defaultLayout,
  parseLayout,
  useLayoutStore,
} from '@/stores/layout'

describe('parseLayout', () => {
  it('gives the defaults for a missing record', () => {
    expect(parseLayout(null)).toEqual(defaultLayout())
  })

  it('gives the defaults for a record it cannot read', () => {
    expect(parseLayout('{not json')).toEqual(defaultLayout())
  })

  it('keeps the values a record holds', () => {
    const stored = {
      panel: 'history',
      panelOpen: false,
      panelWidth: 420,
      editorSize: 60,
      resultsCollapsed: true,
      resultsOrientation: 'beside',
    }
    expect(parseLayout(JSON.stringify(stored))).toEqual(stored)
  })

  it('falls back when the state of the results panel is not a true or false value', () => {
    expect(parseLayout(JSON.stringify({ resultsCollapsed: 'yes' })).resultsCollapsed).toBe(
      defaultLayout().resultsCollapsed,
    )
  })

  it('refuses a place of the results panel it does not know', () => {
    expect(parseLayout(JSON.stringify({ resultsOrientation: 'above' })).resultsOrientation).toBe(
      defaultLayout().resultsOrientation,
    )
  })

  it('holds the width of the side panel inside its limits', () => {
    expect(parseLayout(JSON.stringify({ panelWidth: 40 })).panelWidth).toBe(MIN_PANEL_WIDTH)
    expect(parseLayout(JSON.stringify({ panelWidth: 4000 })).panelWidth).toBe(MAX_PANEL_WIDTH)
  })

  it('falls back when the width is not a number', () => {
    expect(parseLayout(JSON.stringify({ panelWidth: 'wide' })).panelWidth).toBe(
      defaultLayout().panelWidth,
    )
  })

  it('falls back when the open state is not a true or false value', () => {
    expect(parseLayout(JSON.stringify({ panelOpen: 'yes' })).panelOpen).toBe(
      defaultLayout().panelOpen,
    )
  })

  it('refuses a panel it does not know', () => {
    expect(parseLayout(JSON.stringify({ panel: 'nowhere' })).panel).toBe(defaultLayout().panel)
  })

  it('holds the share of the editor inside the limits of the split', () => {
    expect(parseLayout(JSON.stringify({ editorSize: 2 })).editorSize).toBe(MIN_EDITOR_SIZE)
    expect(parseLayout(JSON.stringify({ editorSize: 99 })).editorSize).toBe(MAX_EDITOR_SIZE)
  })

  it('falls back when the share is not a number', () => {
    expect(parseLayout(JSON.stringify({ editorSize: 'wide' })).editorSize).toBe(
      defaultLayout().editorSize,
    )
    expect(parseLayout(JSON.stringify({ editorSize: Number.NaN })).editorSize).toBe(
      defaultLayout().editorSize,
    )
  })
})

describe('useLayoutStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('starts with the defaults', () => {
    expect(useLayoutStore().layout).toEqual(defaultLayout())
  })

  it('reads the shape out of the store it is given', () => {
    const layout = useLayoutStore()
    const storage = { getItem: vi.fn().mockReturnValue(JSON.stringify({ panel: 'explorer' })) }

    layout.load(storage)

    expect(storage.getItem).toHaveBeenCalledWith(LAYOUT_KEY)
    expect(layout.layout.panel).toBe('explorer')
  })

  it('takes the defaults when no store is open to it', () => {
    const layout = useLayoutStore()

    layout.load(null)

    expect(layout.layout).toEqual(defaultLayout())
  })

  it('writes the shape back when the panel changes', () => {
    const layout = useLayoutStore()
    const setItem = vi.spyOn(Storage.prototype, 'setItem')

    layout.showPanel('history')

    expect(layout.layout.panel).toBe('history')
    expect(setItem).toHaveBeenCalledWith(LAYOUT_KEY, JSON.stringify(layout.layout))
    setItem.mockRestore()
  })

  it('writes nothing when no store is open to it', () => {
    const layout = useLayoutStore()

    expect(() => layout.persist(null)).not.toThrow()
  })

  it('holds a new share of the editor inside the limits of the split', () => {
    const layout = useLayoutStore()

    layout.setEditorSize(70)
    expect(layout.layout.editorSize).toBe(70)

    layout.setEditorSize(1)
    expect(layout.layout.editorSize).toBe(MIN_EDITOR_SIZE)

    layout.setEditorSize(150)
    expect(layout.layout.editorSize).toBe(MAX_EDITOR_SIZE)
  })

  it('puts the results panel away and brings it back', () => {
    const layout = useLayoutStore()
    layout.setEditorSize(70)

    layout.toggleResults()
    expect(layout.layout.resultsCollapsed).toBe(true)

    layout.toggleResults()
    expect(layout.layout.resultsCollapsed).toBe(false)
    // The split that the user had comes back with the panel.
    expect(layout.layout.editorSize).toBe(70)

    layout.setResultsCollapsed(true)
    expect(layout.layout.resultsCollapsed).toBe(true)
  })

  it('moves the results panel between the two places it can take', () => {
    const layout = useLayoutStore()
    layout.setEditorSize(70)

    layout.toggleResultsOrientation()
    expect(layout.layout.resultsOrientation).toBe('beside')
    // One share of the editor serves both places.
    expect(layout.layout.editorSize).toBe(70)

    layout.toggleResultsOrientation()
    expect(layout.layout.resultsOrientation).toBe('below')

    layout.setResultsOrientation('beside')
    expect(layout.layout.resultsOrientation).toBe('beside')
  })

  it('keeps the share it has when the new one is not a number', () => {
    const layout = useLayoutStore()
    layout.setEditorSize(60)

    layout.setEditorSize(Number.NaN)

    expect(layout.layout.editorSize).toBe(60)
  })

  it('changes one part of the shape and leaves the rest', () => {
    const layout = useLayoutStore()

    layout.update({ editorSize: 55 })

    expect(layout.layout).toEqual({ ...defaultLayout(), editorSize: 55 })
  })

  it('opens the panel it is asked to show, whatever stood open', () => {
    const layout = useLayoutStore()
    layout.setPanelOpen(false)

    layout.showPanel('explorer')

    expect(layout.layout).toMatchObject({ panel: 'explorer', panelOpen: true })
  })

  it('closes the panel when the rail names the one that already stands open', () => {
    const layout = useLayoutStore()
    layout.showPanel('history')

    layout.selectPanel('history')

    expect(layout.layout.panelOpen).toBe(false)
    expect(layout.layout.panel).toBe('history')
  })

  it('opens the panel again when the rail names it a second time', () => {
    const layout = useLayoutStore()
    layout.showPanel('history')
    layout.selectPanel('history')

    layout.selectPanel('history')

    expect(layout.layout).toMatchObject({ panel: 'history', panelOpen: true })
  })

  it('moves to another panel when the rail names one that is not open', () => {
    const layout = useLayoutStore()
    layout.showPanel('history')

    layout.selectPanel('explorer')

    expect(layout.layout).toMatchObject({ panel: 'explorer', panelOpen: true })
  })

  it('opens a closed panel when the rail names the panel it holds', () => {
    const layout = useLayoutStore()
    layout.showPanel('history')
    layout.setPanelOpen(false)

    layout.selectPanel('history')

    expect(layout.layout).toMatchObject({ panel: 'history', panelOpen: true })
  })

  it('moves between the two states of the panel', () => {
    const layout = useLayoutStore()
    expect(layout.layout.panelOpen).toBe(true)

    layout.togglePanel()
    expect(layout.layout.panelOpen).toBe(false)

    layout.togglePanel()
    expect(layout.layout.panelOpen).toBe(true)
  })

  it('holds a new width of the side panel inside its limits', () => {
    const layout = useLayoutStore()

    layout.setPanelWidth(400)
    expect(layout.layout.panelWidth).toBe(400)

    layout.setPanelWidth(10)
    expect(layout.layout.panelWidth).toBe(MIN_PANEL_WIDTH)

    layout.setPanelWidth(5000)
    expect(layout.layout.panelWidth).toBe(MAX_PANEL_WIDTH)
  })

  it('keeps the width it has when the new one is not a number', () => {
    const layout = useLayoutStore()
    layout.setPanelWidth(400)

    layout.setPanelWidth(Number.NaN)

    expect(layout.layout.panelWidth).toBe(400)
  })

  it('moves the edge of the panel by one step in each direction', () => {
    const layout = useLayoutStore()
    layout.setPanelWidth(400)

    layout.nudgePanelWidth(PANEL_WIDTH_STEP)
    expect(layout.layout.panelWidth).toBe(400 + PANEL_WIDTH_STEP)

    layout.nudgePanelWidth(-PANEL_WIDTH_STEP)
    expect(layout.layout.panelWidth).toBe(400)
  })
})
