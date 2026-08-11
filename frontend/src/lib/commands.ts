/**
 * The command registry. Each command has one identifier, one title, one
 * group and one key. The shell binds the keys and the palette lists the
 * commands, so a new command needs one record and nothing more.
 */

/** One key with the modifiers it needs. */
export interface KeyChord {
  mod: boolean
  shift: boolean
  alt: boolean
  /** The key itself, in small letters. */
  key: string
}

/**
 * Reads a key from a text such as `mod+shift+p`. The word `mod` stands for
 * Control on Windows and on Linux, and for Command on macOS.
 */
export function parseChord(spec: string): KeyChord {
  const parts = spec.toLowerCase().split('+')
  const chord: KeyChord = { mod: false, shift: false, alt: false, key: '' }
  for (const part of parts) {
    if (part === 'mod') {
      chord.mod = true
    } else if (part === 'shift') {
      chord.shift = true
    } else if (part === 'alt') {
      chord.alt = true
    } else {
      chord.key = part
    }
  }
  return chord
}

/**
 * Gives the physical key that the browser reports for a key of a chord. The
 * physical key is used and not the character, because a modifier changes the
 * character on some keyboard layouts.
 */
export function chordCode(key: string): string {
  if (/^[a-z]$/.test(key)) {
    return `Key${key.toUpperCase()}`
  }
  if (/^[0-9]$/.test(key)) {
    return `Digit${key}`
  }
  if (key === ',') {
    return 'Comma'
  }
  return key.charAt(0).toUpperCase() + key.slice(1)
}

/** True when the event carries exactly the chord. */
export function chordMatches(spec: string, event: KeyboardEvent): boolean {
  const chord = parseChord(spec)
  const mod = event.ctrlKey || event.metaKey
  return (
    mod === chord.mod &&
    event.shiftKey === chord.shift &&
    event.altKey === chord.alt &&
    event.code === chordCode(chord.key)
  )
}

/** Writes a chord in the form the user reads on the key list. */
export function chordLabel(spec: string, apple: boolean): string {
  const chord = parseChord(spec)
  const parts: string[] = []
  if (chord.mod) {
    parts.push(apple ? 'Cmd' : 'Ctrl')
  }
  if (chord.shift) {
    parts.push('Shift')
  }
  if (chord.alt) {
    parts.push(apple ? 'Option' : 'Alt')
  }
  parts.push(chord.key.length === 1 ? chord.key.toUpperCase() : chordCode(chord.key))
  return parts.join(' + ')
}

/** One command of the application. */
export interface Command {
  id: string
  title: string
  group: string
  /** The key of the command, or `null` when it has none. */
  key: string | null
  /**
   * The other keys that reach the command. A command of a file holds the
   * key of the desktop beside the key this application grew with, so a user
   * of either habit reaches it. The list of the keys shows `key` alone,
   * which keeps one name for each command.
   */
  aliases?: string[]
  run: () => void
  /** True when the command can run now. A command with no test always can. */
  enabled?: () => boolean
}

/** True when the command can run now. */
export function commandEnabled(command: Command): boolean {
  return command.enabled ? command.enabled() : true
}

/** Every key that reaches one command, its aliases among them. */
export function keysOf(command: Command): string[] {
  return [command.key, ...(command.aliases ?? [])].filter((key): key is string => key !== null)
}

/** Finds the command that the event asks for, if there is one. */
export function commandForEvent(commands: Command[], event: KeyboardEvent): Command | null {
  for (const command of commands) {
    if (keysOf(command).some((key) => chordMatches(key, event))) {
      return command
    }
  }
  return null
}

/**
 * Keeps the commands whose title or group holds every word of the text. An
 * empty text keeps them all.
 */
export function filterCommands(commands: Command[], text: string): Command[] {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) {
    return [...commands]
  }
  return commands.filter((command) => {
    const haystack = `${command.title} ${command.group}`.toLowerCase()
    return words.every((word) => haystack.includes(word))
  })
}

/**
 * What a query view can do on request. The view of each tab records its own
 * actions here, because the shell holds the keys but the view holds the
 * editor.
 */
export interface TabActions {
  runStatement: () => void
  runAll: () => void
  cancel: () => void
  format: () => void
  save: () => void
}

const tabActionRegistry = new Map<string, TabActions>()

/** Records the actions of one tab. */
export function registerTabActions(tabId: string, actions: TabActions): void {
  tabActionRegistry.set(tabId, actions)
}

/** Drops the actions of one tab, which a closed tab does. */
export function forgetTabActions(tabId: string): void {
  tabActionRegistry.delete(tabId)
}

/** Gives the actions of one tab, or `null` when the tab has none. */
export function tabActions(tabId: string | null): TabActions | null {
  return tabId ? (tabActionRegistry.get(tabId) ?? null) : null
}
