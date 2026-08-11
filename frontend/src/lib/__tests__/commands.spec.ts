import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  chordCode,
  chordLabel,
  chordMatches,
  commandEnabled,
  commandForEvent,
  filterCommands,
  forgetTabActions,
  parseChord,
  registerTabActions,
  tabActions,
  type Command,
} from '@/lib/commands'

/** Builds a key event with the parts a chord reads. */
function keyEvent(
  code: string,
  parts: { mod?: boolean; shift?: boolean; alt?: boolean } = {},
): KeyboardEvent {
  return new KeyboardEvent('keydown', {
    code,
    ctrlKey: parts.mod ?? false,
    shiftKey: parts.shift ?? false,
    altKey: parts.alt ?? false,
  })
}

function commandFor(key: string | null, run = vi.fn()): Command {
  return { id: `c-${key}`, title: 'Do the work', group: 'Test', key, run }
}

describe('parseChord', () => {
  it('reads the modifiers and the key', () => {
    expect(parseChord('mod+shift+alt+p')).toEqual({ mod: true, shift: true, alt: true, key: 'p' })
    expect(parseChord('f1')).toEqual({ mod: false, shift: false, alt: false, key: 'f1' })
  })
})

describe('chordCode', () => {
  it('names the physical key', () => {
    expect(chordCode('f')).toBe('KeyF')
    expect(chordCode('2')).toBe('Digit2')
    expect(chordCode(',')).toBe('Comma')
    expect(chordCode('enter')).toBe('Enter')
    expect(chordCode('f1')).toBe('F1')
  })
})

describe('chordMatches', () => {
  it('accepts the key with exactly its modifiers', () => {
    expect(chordMatches('mod+enter', keyEvent('Enter', { mod: true }))).toBe(true)
    expect(chordMatches('mod+shift+p', keyEvent('KeyP', { mod: true, shift: true }))).toBe(true)
    expect(chordMatches('shift+alt+f', keyEvent('KeyF', { shift: true, alt: true }))).toBe(true)
    expect(chordMatches('f1', keyEvent('F1'))).toBe(true)
  })

  it('refuses the key with any other modifiers', () => {
    expect(chordMatches('mod+enter', keyEvent('Enter'))).toBe(false)
    expect(chordMatches('mod+enter', keyEvent('Enter', { mod: true, shift: true }))).toBe(false)
    expect(chordMatches('mod+enter', keyEvent('Enter', { mod: true, alt: true }))).toBe(false)
    expect(chordMatches('mod+enter', keyEvent('KeyT', { mod: true }))).toBe(false)
  })

  it('takes the Command key of macOS for the modifier', () => {
    const event = new KeyboardEvent('keydown', { code: 'KeyT', metaKey: true })
    expect(chordMatches('mod+t', event)).toBe(true)
  })
})

describe('chordLabel', () => {
  it('writes the key for a keyboard that has a Control key', () => {
    expect(chordLabel('mod+shift+alt+f', false)).toBe('Ctrl + Shift + Alt + F')
    expect(chordLabel('mod+1', false)).toBe('Ctrl + 1')
    expect(chordLabel('f1', false)).toBe('F1')
  })

  it('names the keys of an Apple keyboard', () => {
    expect(chordLabel('mod+alt+f', true)).toBe('Cmd + Option + F')
  })
})

describe('commandEnabled', () => {
  it('holds a command that gives no test to be ready', () => {
    expect(commandEnabled(commandFor('mod+t'))).toBe(true)
  })

  it('asks the test of a command that gives one', () => {
    expect(commandEnabled({ ...commandFor('mod+t'), enabled: () => false })).toBe(false)
  })
})

describe('commandForEvent', () => {
  it('finds the command whose key the event carries', () => {
    const wanted = commandFor('mod+t')
    const commands = [commandFor(null), commandFor('mod+w'), wanted]
    expect(commandForEvent(commands, keyEvent('KeyT', { mod: true }))).toBe(wanted)
  })

  it('finds nothing for a key that no command holds', () => {
    expect(commandForEvent([commandFor('mod+w')], keyEvent('KeyZ', { mod: true }))).toBeNull()
  })
})

describe('filterCommands', () => {
  const commands = [
    { ...commandFor('mod+t'), title: 'New tab', group: 'Tabs' },
    { ...commandFor('mod+1'), title: 'Show the connections', group: 'View' },
  ]

  it('keeps every command for an empty text', () => {
    expect(filterCommands(commands, '   ')).toHaveLength(2)
  })

  it('keeps the commands that hold every word', () => {
    expect(filterCommands(commands, 'show view').map((command) => command.title)).toEqual([
      'Show the connections',
    ])
    expect(filterCommands(commands, 'show tabs')).toEqual([])
  })
})

describe('the actions of a tab', () => {
  const actions = {
    runStatement: vi.fn(),
    runAll: vi.fn(),
    cancel: vi.fn(),
    format: vi.fn(),
    save: vi.fn(),
  }

  beforeEach(() => {
    forgetTabActions('t1')
  })

  it('gives the actions the tab recorded', () => {
    registerTabActions('t1', actions)
    expect(tabActions('t1')).toBe(actions)
  })

  it('gives nothing for a tab that recorded none', () => {
    expect(tabActions('t1')).toBeNull()
    expect(tabActions(null)).toBeNull()
  })

  it('gives nothing after the tab is forgotten', () => {
    registerTabActions('t1', actions)
    forgetTabActions('t1')
    expect(tabActions('t1')).toBeNull()
  })
})
