import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import {
  SETTINGS_KEY,
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

  it('moves between the two themes', () => {
    const settings = useSettingsStore()
    expect(settings.isDark).toBe(true)
    expect(settings.editorTheme).toBe('sql-explorer-dark')
    settings.toggleTheme()
    expect(settings.settings.theme).toBe('sqlExplorerLight')
    settings.toggleTheme()
    expect(settings.settings.theme).toBe('sqlExplorerDark')
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
