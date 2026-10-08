import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { ERROR_TIMEOUT_MS, MAX_NOTICES, useUiStore } from '@/stores/ui'
import { ErrorCategory } from '@/types/api'

describe('ui store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('starts with no notices', () => {
    const ui = useUiStore()
    expect(ui.notices).toEqual([])
    expect(ui.openedNotice).toBeNull()
  })

  it('adds a notice for each level', () => {
    const ui = useUiStore()
    ui.success('done')
    ui.info('note')
    ui.warn('careful')
    expect(ui.notices.map((notice) => notice.level)).toEqual(['success', 'info', 'warning'])
    expect(ui.notices[0]?.timeout).toBe(3000)
    expect(ui.notices[1]?.timeout).toBe(4000)
    expect(ui.notices[2]?.timeout).toBe(6000)
  })

  it('keeps a warning that the caller asks to keep, also past the limit', () => {
    const ui = useUiStore()
    const kept = ui.warn('storage', 'detail', { kept: true })
    expect(kept.timeout).toBe(-1)
    expect(kept.detail).toBe('detail')
    // Short notices that arrive later leave first, so the kept one stays.
    for (let index = 0; index < MAX_NOTICES + 2; index += 1) {
      ui.success(`Notice ${index}`)
    }
    expect(ui.notices[0]?.id).toBe(kept.id)
  })

  it('puts a button on a warning only when the caller gives one', () => {
    const ui = useUiStore()
    const run = vi.fn()
    expect(ui.warn('failed', null, { action: { label: 'Try again', run } }).action?.label).toBe(
      'Try again',
    )
    expect('action' in ui.warn('plain')).toBe(false)
  })

  it('gives every notice its own identifier', () => {
    const ui = useUiStore()
    const first = ui.success('a')
    const second = ui.success('b')
    expect(first.id).not.toBe(second.id)
  })

  it('removes one notice and keeps the rest', () => {
    const ui = useUiStore()
    const first = ui.success('a')
    ui.success('b')
    ui.dismiss(first.id)
    expect(ui.notices).toHaveLength(1)
    expect(ui.notices[0]?.message).toBe('b')
  })

  it('closes the opened notice when that notice is removed', () => {
    const ui = useUiStore()
    const notice = ui.success('a')
    ui.openNotice(notice)
    expect(ui.openedNotice?.id).toBe(notice.id)
    ui.dismiss(notice.id)
    expect(ui.openedNotice).toBeNull()
  })

  it('keeps the opened notice when another notice is removed', () => {
    const ui = useUiStore()
    const kept = ui.success('a')
    const other = ui.success('b')
    ui.openNotice(kept)
    ui.dismiss(other.id)
    expect(ui.openedNotice?.id).toBe(kept.id)
  })

  it('removes every notice at once', () => {
    const ui = useUiStore()
    ui.success('a')
    ui.openNotice(ui.notices[0]!)
    ui.clear()
    expect(ui.notices).toEqual([])
    expect(ui.openedNotice).toBeNull()
  })

  it('closes the opened notice on request', () => {
    const ui = useUiStore()
    ui.openNotice(ui.success('a'))
    ui.closeNotice()
    expect(ui.openedNotice).toBeNull()
  })

  it('reports a failure as an error that stays on screen', () => {
    const ui = useUiStore()
    const payload = ui.reportError({
      category: ErrorCategory.Connection,
      message: 'the host refused',
      detail: 'socket closed',
    })
    expect(payload.category).toBe(ErrorCategory.Connection)
    expect(ui.notices).toHaveLength(1)
    expect(ui.notices[0]?.level).toBe('error')
    expect(ui.notices[0]?.timeout).toBe(-1)
    expect(ui.notices[0]?.detail).toContain("Check the connection's host")
    expect(ui.notices[0]?.detail).toContain('socket closed')
  })

  it('leaves the detail empty when there is no advice and no cause', () => {
    const ui = useUiStore()
    ui.reportError({ category: ErrorCategory.Database, message: 'bad column', detail: null })
    expect(ui.notices[0]?.detail).toBeNull()
  })

  it('reports a stopped statement as a note and not as an error', () => {
    const ui = useUiStore()
    ui.reportError({ category: ErrorCategory.Cancelled, message: 'stopped', detail: null })
    expect(ui.notices).toHaveLength(1)
    expect(ui.notices[0]?.level).toBe('info')
  })

  it('exposes the helper that joins a message and its cause', () => {
    const ui = useUiStore()
    expect(ui.fullErrorText({ category: ErrorCategory.Database, message: 'a', detail: 'b' })).toBe(
      'a\nb',
    )
  })
})

describe('ui store dialog count', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('starts with no dialog open', () => {
    const ui = useUiStore()
    expect(ui.openDialogs).toBe(0)
    expect(ui.dialogOpen).toBe(false)
  })

  it('counts each dialog that opens and each that closes', () => {
    const ui = useUiStore()

    ui.addDialog()
    ui.addDialog()
    expect(ui.dialogOpen).toBe(true)

    ui.removeDialog()
    expect(ui.dialogOpen).toBe(true)

    ui.removeDialog()
    expect(ui.dialogOpen).toBe(false)
  })

  it('never counts below none', () => {
    const ui = useUiStore()

    ui.removeDialog()

    expect(ui.openDialogs).toBe(0)
  })
})

describe('ui store overlays', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('opens and closes the palette, the key list and the guide', () => {
    const ui = useUiStore()
    expect(ui.paletteOpen).toBe(false)
    expect(ui.keyboardHelpOpen).toBe(false)
    expect(ui.guideOpen).toBe(false)

    ui.setPaletteOpen(true)
    ui.setKeyboardHelpOpen(true)
    ui.setGuideOpen(true)
    expect(ui.paletteOpen).toBe(true)
    expect(ui.keyboardHelpOpen).toBe(true)
    expect(ui.guideOpen).toBe(true)

    ui.setGuideOpen(false)
    expect(ui.guideOpen).toBe(false)
  })
})

describe('ui store holding the corner to a few notices', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('keeps only the newest notices when a burst arrives', () => {
    const ui = useUiStore()

    for (let index = 0; index < MAX_NOTICES + 3; index += 1) {
      ui.info(`Notice ${index}`)
    }

    expect(ui.notices).toHaveLength(MAX_NOTICES)
    // The oldest go, so the newest are the ones the user can still read.
    expect(ui.notices[0]?.message).toBe('Notice 3')
    expect(ui.notices[ui.notices.length - 1]?.message).toBe(`Notice ${MAX_NOTICES + 2}`)
  })

  it('keeps an error that stays while a burst of short notices arrives', () => {
    const ui = useUiStore()

    ui.reportError({ category: ErrorCategory.Database, message: 'It failed', detail: null })
    for (let index = 0; index < MAX_NOTICES + 3; index += 1) {
      ui.info(`Notice ${index}`)
    }

    expect(ui.notices).toHaveLength(MAX_NOTICES)
    expect(ui.notices[0]?.message).toBe('It failed')
    expect(ui.notices[ui.notices.length - 1]?.message).toBe(`Notice ${MAX_NOTICES + 2}`)
  })

  it('loses its oldest notice when every notice stays', () => {
    const ui = useUiStore()

    for (let index = 0; index < MAX_NOTICES + 1; index += 1) {
      ui.reportError({ category: ErrorCategory.Database, message: `Fault ${index}`, detail: null })
    }

    expect(ui.notices).toHaveLength(MAX_NOTICES)
    expect(ui.notices[0]?.message).toBe('Fault 1')
  })

  it('holds an error in the corner until the user takes it away', () => {
    const ui = useUiStore()

    ui.reportError({ category: ErrorCategory.Database, message: 'It failed', detail: null })

    expect(ui.notices[0]?.timeout).toBe(-1)
  })

  it('lets an error leave on its own when the same words are kept elsewhere', () => {
    const ui = useUiStore()

    ui.reportError(
      { category: ErrorCategory.Database, message: 'It failed', detail: null },
      { kept: true },
    )

    expect(ui.notices[0]?.timeout).toBe(ERROR_TIMEOUT_MS)
  })
})
