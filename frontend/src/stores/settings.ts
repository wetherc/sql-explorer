import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

/** The two themes the application draws itself with. */
export type ThemeName = 'sqlExplorerDark' | 'sqlExplorerLight'

/**
 * What the user asked for. The `system` choice takes the theme from the host,
 * and follows it when the user changes it there.
 */
export type ThemeChoice = ThemeName | 'system'

/** The choices the settings offer, in the order the dialog shows them. */
export const THEME_CHOICES: Array<{ title: string; value: ThemeChoice }> = [
  { title: 'Follow the system', value: 'system' },
  { title: 'Dark', value: 'sqlExplorerDark' },
  { title: 'Light', value: 'sqlExplorerLight' },
]

/** The media rule that asks the host for a dark theme. */
export const DARK_MEDIA_QUERY = '(prefers-color-scheme: dark)'

export interface Settings {
  theme: ThemeChoice
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
  /**
   * The largest number of columns the read of a schema keeps. A catalog that
   * holds more gives a part of itself, so the memory stays bounded.
   */
  schemaSnapshotColumns: number
  /**
   * True when the read of a schema opens a second connection of its own. The
   * read then never waits behind a statement of the user, at the cost of one
   * more session on the server. A false value puts the read on the one
   * session, and a statement of the user then waits for it.
   */
  schemaSnapshotOwnConnection: boolean
}

/** The settings a new installation starts with. */
export function defaultSettings(): Settings {
  return {
    theme: 'system',
    fontSize: 13,
    wordWrap: false,
    showLineNumbers: true,
    maxRows: 10000,
    autoRunPreview: true,
    maxPinnedResults: 5,
    exportRowLimit: 1000000,
    athenaPricePerTerabyte: 5,
    athenaScanWarningGb: 100,
    schemaSnapshotColumns: 20000,
    schemaSnapshotOwnConnection: true,
  }
}

/** The key under which the settings live in the browser store. */
export const SETTINGS_KEY = 'sql-explorer.settings'

/**
 * Holds every value of the settings inside the bounds that belong to it, and
 * falls back on the value it is given for anything it cannot read.
 *
 * The read of a stored record and the change of one setting both come through
 * here, so a figure typed into the dialog meets the same bounds as a figure
 * read from the browser store. A field left empty or filled with letters
 * reaches this as `NaN`, and the fallback then keeps the value that stood
 * before it.
 */
export function normaliseSettings(candidate: Partial<Settings>, fallback: Settings): Settings {
  return {
    theme: THEME_CHOICES.some((choice) => choice.value === candidate.theme)
      ? (candidate.theme as ThemeChoice)
      : fallback.theme,
    fontSize: numberOr(candidate.fontSize, fallback.fontSize, 8, 32),
    wordWrap: typeof candidate.wordWrap === 'boolean' ? candidate.wordWrap : fallback.wordWrap,
    showLineNumbers:
      typeof candidate.showLineNumbers === 'boolean'
        ? candidate.showLineNumbers
        : fallback.showLineNumbers,
    maxRows: numberOr(candidate.maxRows, fallback.maxRows, 1, 1000000),
    autoRunPreview:
      typeof candidate.autoRunPreview === 'boolean'
        ? candidate.autoRunPreview
        : fallback.autoRunPreview,
    maxPinnedResults: numberOr(candidate.maxPinnedResults, fallback.maxPinnedResults, 1, 20),
    exportRowLimit: numberOr(candidate.exportRowLimit, fallback.exportRowLimit, 1000, 100000000),
    // The price keeps its fraction, so it does not go through `numberOr`.
    athenaPricePerTerabyte: priceOr(
      candidate.athenaPricePerTerabyte,
      fallback.athenaPricePerTerabyte,
    ),
    athenaScanWarningGb: numberOr(
      candidate.athenaScanWarningGb,
      fallback.athenaScanWarningGb,
      1,
      1000000,
    ),
    schemaSnapshotColumns: numberOr(
      candidate.schemaSnapshotColumns,
      fallback.schemaSnapshotColumns,
      100,
      200000,
    ),
    schemaSnapshotOwnConnection:
      typeof candidate.schemaSnapshotOwnConnection === 'boolean'
        ? candidate.schemaSnapshotOwnConnection
        : fallback.schemaSnapshotOwnConnection,
  }
}

/** Reads the settings and falls back on the defaults for a bad record. */
export function parseSettings(raw: string | null): Settings {
  const defaults = defaultSettings()
  if (!raw) {
    return defaults
  }
  try {
    return normaliseSettings(JSON.parse(raw) as Partial<Settings>, defaults)
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
  /** True while the host asks for a dark theme. */
  const systemPrefersDark = ref(false)

  /**
   * The theme the application draws itself with. It is the choice of the user,
   * or the theme of the host when the user asked to follow it.
   */
  const resolvedTheme = computed<ThemeName>(() => {
    if (settings.value.theme === 'system') {
      return systemPrefersDark.value ? 'sqlExplorerDark' : 'sqlExplorerLight'
    }
    return settings.value.theme
  })

  const isDark = computed(() => resolvedTheme.value === 'sqlExplorerDark')
  /** The theme the editor uses, which follows the theme of the application. */
  const editorTheme = computed(() => (isDark.value ? 'sql-explorer-dark' : 'sql-explorer-light'))

  function load(storage: Pick<Storage, 'getItem'> | null = safeStorage()): void {
    settings.value = parseSettings(storage ? storage.getItem(SETTINGS_KEY) : null)
  }

  function persist(storage: Pick<Storage, 'setItem'> | null = safeStorage()): void {
    storage?.setItem(SETTINGS_KEY, JSON.stringify(settings.value))
  }

  /**
   * Changes part of the settings. A value outside its bounds is brought inside
   * them, and a value that cannot be read at all leaves the setting as it was.
   */
  function update(patch: Partial<Settings>): void {
    settings.value = normaliseSettings({ ...settings.value, ...patch }, settings.value)
    persist()
  }

  /** Puts every setting back to the value a new installation starts with. */
  function reset(): void {
    settings.value = defaultSettings()
    persist()
  }

  /**
   * Moves to the theme opposite the one on screen. A user who followed the
   * host and then asks for the other theme names a theme of their own, because
   * following the host is what they left behind.
   */
  function toggleTheme(): void {
    update({ theme: isDark.value ? 'sqlExplorerLight' : 'sqlExplorerDark' })
  }

  /**
   * Follows the theme of the host and reports each change of it. The caller
   * gets back a function that stops the watch.
   */
  function watchSystemTheme(query: MediaQueryList | null = darkMediaQuery()): () => void {
    if (!query) {
      return () => {}
    }
    systemPrefersDark.value = query.matches
    const onChange = (event: MediaQueryListEvent): void => {
      systemPrefersDark.value = event.matches
    }
    query.addEventListener('change', onChange)
    return () => query.removeEventListener('change', onChange)
  }

  return {
    settings,
    systemPrefersDark,
    resolvedTheme,
    isDark,
    editorTheme,
    load,
    persist,
    update,
    reset,
    toggleTheme,
    watchSystemTheme,
  }
})

/** Asks the host whether it wants a dark theme, or nothing when it cannot say. */
export function darkMediaQuery(): MediaQueryList | null {
  try {
    if (typeof globalThis.matchMedia !== 'function') {
      return null
    }
    return globalThis.matchMedia(DARK_MEDIA_QUERY)
  } catch {
    return null
  }
}

/** Returns the browser store, or nothing when the host forbids it. */
export function safeStorage(): Storage | null {
  try {
    return globalThis.localStorage ?? null
  } catch {
    return null
  }
}
