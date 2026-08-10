import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import {
  LAYOUT_KEY,
  MAX_EDITOR_SIZE,
  MIN_EDITOR_SIZE,
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
    expect(parseLayout(JSON.stringify({ panel: 'history', editorSize: 60 }))).toEqual({
      panel: 'history',
      editorSize: 60,
    })
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

  it('keeps the share it has when the new one is not a number', () => {
    const layout = useLayoutStore()
    layout.setEditorSize(60)

    layout.setEditorSize(Number.NaN)

    expect(layout.layout.editorSize).toBe(60)
  })

  it('changes one part of the shape and leaves the rest', () => {
    const layout = useLayoutStore()

    layout.update({ editorSize: 55 })

    expect(layout.layout).toEqual({ panel: defaultLayout().panel, editorSize: 55 })
  })
})
