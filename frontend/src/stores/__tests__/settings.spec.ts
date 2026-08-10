import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import {
  DARK_MEDIA_QUERY,
  SETTINGS_KEY,
  darkMediaQuery,
  defaultSettings,
  parseSettings,
  safeStorage,
  useSettingsStore,
} from '@/stores/settings'

describe('parseSettings', () => {
  it('gives the defaults for a missing record', () => {
    expect(parseSettings(null)).toEqual(defaultSettings())
  })

  it('gives the defaults for a record it cannot read', () => {
    expect(parseSettings('{not json')).toEqual(defaultSettings())
  })

  it('keeps the values a record holds', () => {
    const stored = JSON.stringify({
      theme: 'sqlExplorerLight',
      fontSize: 18,
      wordWrap: true,
      showLineNumbers: false,
      maxRows: 500,
      autoRunPreview: false,
      maxPinnedResults: 8,
      exportRowLimit: 2000,
      athenaPricePerTerabyte: 6.25,
      athenaScanWarningGb: 50,
      schemaSnapshotColumns: 40000,
      schemaSnapshotOwnConnection: false,
    })
    expect(parseSettings(stored)).toEqual({
      theme: 'sqlExplorerLight',
      fontSize: 18,
      wordWrap: true,
      showLineNumbers: false,
      maxRows: 500,
      autoRunPreview: false,
      maxPinnedResults: 8,
      exportRowLimit: 2000,
      athenaPricePerTerabyte: 6.25,
      athenaScanWarningGb: 50,
      schemaSnapshotColumns: 40000,
      schemaSnapshotOwnConnection: false,
    })
  })

  it('falls back for a field that is missing or of the wrong type', () => {
    const parsed = parseSettings(
      JSON.stringify({ theme: 'other', fontSize: 'big', wordWrap: 1, maxRows: null }),
    )
    expect(parsed).toEqual(defaultSettings())
  })

  it('keeps a number inside its limits', () => {
    expect(parseSettings(JSON.stringify({ fontSize: 200 })).fontSize).toBe(32)
    expect(parseSettings(JSON.stringify({ fontSize: 1 })).fontSize).toBe(8)
    expect(parseSettings(JSON.stringify({ fontSize: 12.6 })).fontSize).toBe(13)
    expect(parseSettings(JSON.stringify({ maxRows: 0 })).maxRows).toBe(1)
    expect(parseSettings(JSON.stringify({ fontSize: Number.NaN })).fontSize).toBe(13)
  })
})

describe('settings store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('reads and writes through a store the caller gives', () => {
    const settings = useSettingsStore()
    const getItem = vi.fn(() => JSON.stringify({ theme: 'sqlExplorerLight' }))
    settings.load({ getItem })
    expect(getItem).toHaveBeenCalledWith(SETTINGS_KEY)
    expect(settings.isDark).toBe(false)
    expect(settings.editorTheme).toBe('sql-explorer-light')

    const setItem = vi.fn()
    settings.persist({ setItem })
    expect(setItem).toHaveBeenCalledWith(SETTINGS_KEY, expect.stringContaining('sqlExplorerLight'))
  })

  it('falls back to the defaults when there is no store', () => {
    const settings = useSettingsStore()
    settings.load(null)
    expect(settings.settings).toEqual(defaultSettings())
    settings.persist(null)
  })

  it('changes one field and keeps the rest', () => {
    const settings = useSettingsStore()
    settings.update({ fontSize: 20 })
    expect(settings.settings.fontSize).toBe(20)
    expect(settings.settings.maxRows).toBe(defaultSettings().maxRows)
  })

  it('moves to the theme opposite the one on screen', () => {
    const settings = useSettingsStore()
    // A new installation follows the host, which reports a light theme here.
    expect(settings.settings.theme).toBe('system')
    expect(settings.isDark).toBe(false)
    expect(settings.editorTheme).toBe('sql-explorer-light')

    settings.toggleTheme()
    expect(settings.settings.theme).toBe('sqlExplorerDark')
    expect(settings.editorTheme).toBe('sql-explorer-dark')

    settings.toggleTheme()
    expect(settings.settings.theme).toBe('sqlExplorerLight')
  })

  it('takes the theme of the host while the choice is to follow it', () => {
    const settings = useSettingsStore()

    settings.systemPrefersDark = true
    expect(settings.resolvedTheme).toBe('sqlExplorerDark')
    expect(settings.isDark).toBe(true)

    settings.systemPrefersDark = false
    expect(settings.resolvedTheme).toBe('sqlExplorerLight')
  })

  it('holds to the theme the user named, whatever the host reports', () => {
    const settings = useSettingsStore()
    settings.update({ theme: 'sqlExplorerLight' })

    settings.systemPrefersDark = true

    expect(settings.resolvedTheme).toBe('sqlExplorerLight')
    expect(settings.isDark).toBe(false)
  })

  it('follows the host and reports each change of its theme', () => {
    const settings = useSettingsStore()
    const listeners: Array<(event: MediaQueryListEvent) => void> = []
    const query = {
      matches: true,
      addEventListener: vi.fn((_name: string, handler: (event: MediaQueryListEvent) => void) => {
        listeners.push(handler)
      }),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList

    const stop = settings.watchSystemTheme(query)
    expect(settings.systemPrefersDark).toBe(true)

    listeners[0]!({ matches: false } as MediaQueryListEvent)
    expect(settings.systemPrefersDark).toBe(false)

    stop()
    expect(query.removeEventListener).toHaveBeenCalled()
  })

  it('leaves the theme of the host alone when it cannot be asked', () => {
    const settings = useSettingsStore()

    const stop = settings.watchSystemTheme(null)

    expect(settings.systemPrefersDark).toBe(false)
    expect(() => stop()).not.toThrow()
  })
})

describe('darkMediaQuery', () => {
  it('gives the media rule of the host', () => {
    expect(darkMediaQuery()).not.toBeNull()
    expect(window.matchMedia).toHaveBeenCalledWith(DARK_MEDIA_QUERY)
  })

  it('gives nothing when the host cannot answer questions about its media', () => {
    const original = window.matchMedia
    Object.defineProperty(window, 'matchMedia', { writable: true, value: undefined })

    expect(darkMediaQuery()).toBeNull()

    Object.defineProperty(window, 'matchMedia', { writable: true, value: original })
  })

  it('gives nothing when the host refuses the question', () => {
    const matchMedia = vi.spyOn(window, 'matchMedia').mockImplementation(() => {
      throw new Error('no')
    })

    expect(darkMediaQuery()).toBeNull()
    matchMedia.mockRestore()
  })
})

describe('settings store without a browser store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('changes a field even when nothing can be written', () => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      get() {
        throw new Error('forbidden')
      },
    })
    const settings = useSettingsStore()
    settings.update({ fontSize: 15 })
    expect(settings.settings.fontSize).toBe(15)
    if (descriptor) {
      Object.defineProperty(globalThis, 'localStorage', descriptor)
    }
  })
})

describe('safeStorage', () => {
  it('gives the store of the host', () => {
    expect(safeStorage()).toBe(globalThis.localStorage)
  })

  it('gives nothing when the host forbids the store', () => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      get() {
        throw new Error('forbidden')
      },
    })
    expect(safeStorage()).toBeNull()
    if (descriptor) {
      Object.defineProperty(globalThis, 'localStorage', descriptor)
    }
  })
})

describe('safeStorage without a store', () => {
  it('gives nothing when the host offers no store', () => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: undefined })
    expect(safeStorage()).toBeNull()
    if (descriptor) {
      Object.defineProperty(globalThis, 'localStorage', descriptor)
    }
  })
})
