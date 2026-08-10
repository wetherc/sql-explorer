import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

export type ThemeName = 'sqlExplorerDark' | 'sqlExplorerLight'

export interface Settings {
  theme: ThemeName
  fontSize: number
  wordWrap: boolean
  showLineNumbers: boolean
  /** The largest number of rows one result set holds. */
  maxRows: number
  /** True when a statement runs as soon as the user opens a preview. */
  autoRunPreview: boolean
  /** The largest number of results one tab keeps against the next run. */
  maxPinnedResults: number
  /** The row limit of an export that writes straight to a file. */
  exportRowLimit: number
  /**
   * The price of one terabyte that Athena scans, in US dollars. The rate
   * changes by region and by contract, so the figure is an estimate.
   */
  athenaPricePerTerabyte: number
  /** A scan above this size in gigabytes raises a warning. */
  athenaScanWarningGb: number
}

/** The settings a new installation starts with. */
export function defaultSettings(): Settings {
  return {
    theme: 'sqlExplorerDark',
    fontSize: 13,
    wordWrap: false,
    showLineNumbers: true,
    maxRows: 10000,
    autoRunPreview: true,
    maxPinnedResults: 5,
    exportRowLimit: 1000000,
    athenaPricePerTerabyte: 5,
    athenaScanWarningGb: 100,
  }
}

/** The key under which the settings live in the browser store. */
export const SETTINGS_KEY = 'sql-explorer.settings'

/** Reads the settings and falls back on the defaults for a bad record. */
export function parseSettings(raw: string | null): Settings {
  const defaults = defaultSettings()
  if (!raw) {
    return defaults
  }
  try {
    const parsed = JSON.parse(raw) as Partial<Settings>
    return {
      theme: parsed.theme === 'sqlExplorerLight' ? 'sqlExplorerLight' : defaults.theme,
      fontSize: numberOr(parsed.fontSize, defaults.fontSize, 8, 32),
      wordWrap: typeof parsed.wordWrap === 'boolean' ? parsed.wordWrap : defaults.wordWrap,
      showLineNumbers:
        typeof parsed.showLineNumbers === 'boolean'
          ? parsed.showLineNumbers
          : defaults.showLineNumbers,
      maxRows: numberOr(parsed.maxRows, defaults.maxRows, 1, 1000000),
      autoRunPreview:
        typeof parsed.autoRunPreview === 'boolean'
          ? parsed.autoRunPreview
          : defaults.autoRunPreview,
      maxPinnedResults: numberOr(parsed.maxPinnedResults, defaults.maxPinnedResults, 1, 20),
      exportRowLimit: numberOr(parsed.exportRowLimit, defaults.exportRowLimit, 1000, 100000000),
      // The price keeps its fraction, so it does not go through `numberOr`.
      athenaPricePerTerabyte: priceOr(
        parsed.athenaPricePerTerabyte,
        defaults.athenaPricePerTerabyte,
      ),
      athenaScanWarningGb: numberOr(
        parsed.athenaScanWarningGb,
        defaults.athenaScanWarningGb,
        1,
        1000000,
      ),
    }
  } catch {
    return defaults
  }
}

/** Keeps a price at or above zero, and falls back when it is not a number. */
function priceOr(value: unknown, fallback: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
    return fallback
  }
  return value
}

/** Keeps a number inside its limits, and falls back when it is not a number. */
function numberOr(value: unknown, fallback: number, low: number, high: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return fallback
  }
  return Math.min(high, Math.max(low, Math.round(value)))
}

export const useSettingsStore = defineStore('settings', () => {
  const settings = ref<Settings>(defaultSettings())

  const isDark = computed(() => settings.value.theme === 'sqlExplorerDark')
  /** The theme the editor uses, which follows the theme of the application. */
  const editorTheme = computed(() => (isDark.value ? 'sql-explorer-dark' : 'sql-explorer-light'))

  function load(storage: Pick<Storage, 'getItem'> | null = safeStorage()): void {
    settings.value = parseSettings(storage ? storage.getItem(SETTINGS_KEY) : null)
  }

  function persist(storage: Pick<Storage, 'setItem'> | null = safeStorage()): void {
    storage?.setItem(SETTINGS_KEY, JSON.stringify(settings.value))
  }

  function update(patch: Partial<Settings>): void {
    settings.value = { ...settings.value, ...patch }
    persist()
  }

  function toggleTheme(): void {
    update({ theme: isDark.value ? 'sqlExplorerLight' : 'sqlExplorerDark' })
  }

  return { settings, isDark, editorTheme, load, persist, update, toggleTheme }
})

/** Returns the browser store, or nothing when the host forbids it. */
export function safeStorage(): Storage | null {
  try {
    return globalThis.localStorage ?? null
  } catch {
    return null
  }
}
