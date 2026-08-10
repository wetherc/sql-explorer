import { describe, expect, it } from 'vitest'
import {
  NULL_TEXT,
  compareCells,
  formatCell,
  formatDuration,
  formatRowCount,
  formatTimestamp,
  isNullCell,
  summariseQuery,
  truncate,
} from '@/lib/format'

describe('formatCell', () => {
  it('names a cell that holds no value', () => {
    expect(formatCell(null)).toBe(NULL_TEXT)
    expect(formatCell(undefined as unknown as null)).toBe(NULL_TEXT)
  })

  it('keeps a text as it is', () => {
    expect(formatCell('abc')).toBe('abc')
    expect(formatCell('')).toBe('')
  })

  it('writes numbers and switches as text', () => {
    expect(formatCell(42)).toBe('42')
    expect(formatCell(true)).toBe('true')
    expect(formatCell(false)).toBe('false')
  })

  it('writes a structured value as JSON', () => {
    expect(formatCell({ a: 1 })).toBe('{"a":1}')
    expect(formatCell([1, 2])).toBe('[1,2]')
  })
})

describe('isNullCell', () => {
  it('holds only for a cell without a value', () => {
    expect(isNullCell(null)).toBe(true)
    expect(isNullCell(undefined as unknown as null)).toBe(true)
    expect(isNullCell('')).toBe(false)
    expect(isNullCell(0)).toBe(false)
  })
})

describe('truncate', () => {
  it('leaves a short text alone', () => {
    expect(truncate('abc', 5)).toBe('abc')
    expect(truncate('abcde', 5)).toBe('abcde')
  })

  it('cuts a long text and marks the cut', () => {
    expect(truncate('abcdef', 3)).toBe('abc…')
  })

  it('leaves the text alone when the limit is not positive', () => {
    expect(truncate('abcdef', 0)).toBe('abcdef')
    expect(truncate('abcdef', -1)).toBe('abcdef')
  })

  it('uses a limit of its own when none is given', () => {
    expect(truncate('a'.repeat(300))).toHaveLength(201)
  })
})

describe('formatDuration', () => {
  it('uses milliseconds below one second', () => {
    expect(formatDuration(0)).toBe('0 ms')
    expect(formatDuration(999)).toBe('999 ms')
    expect(formatDuration(12.4)).toBe('12 ms')
  })

  it('uses seconds below one minute', () => {
    expect(formatDuration(1500)).toBe('1.50 s')
    expect(formatDuration(59_000)).toBe('59.00 s')
  })

  it('uses minutes above one minute', () => {
    expect(formatDuration(60_000)).toBe('1 min 0 s')
    expect(formatDuration(125_000)).toBe('2 min 5 s')
  })
})

describe('formatRowCount', () => {
  it('uses the correct word', () => {
    expect(formatRowCount(1)).toBe('1 row')
    expect(formatRowCount(0)).toBe('0 rows')
    expect(formatRowCount(2)).toBe('2 rows')
  })
})

describe('compareCells', () => {
  it('puts a cell without a value at the end', () => {
    expect(compareCells(null, null)).toBe(0)
    expect(compareCells(null, 1)).toBe(1)
    expect(compareCells(1, null)).toBe(-1)
  })

  it('compares numbers as numbers', () => {
    expect(compareCells(2, 10)).toBeLessThan(0)
  })

  it('compares switches', () => {
    expect(compareCells(false, true)).toBeLessThan(0)
    expect(compareCells(true, false)).toBeGreaterThan(0)
  })

  it('compares everything else as text', () => {
    expect(compareCells('a', 'b')).toBeLessThan(0)
    expect(compareCells('item2', 'item10')).toBeLessThan(0)
    expect(compareCells({ a: 1 }, { a: 1 })).toBe(0)
  })
})

describe('formatTimestamp', () => {
  it('writes a moment in the local form', () => {
    expect(formatTimestamp('2026-08-10T12:00:00Z')).not.toBe('2026-08-10T12:00:00Z')
  })

  it('keeps a value it cannot read', () => {
    expect(formatTimestamp('not a moment')).toBe('not a moment')
  })
})

describe('summariseQuery', () => {
  it('folds a statement onto one line', () => {
    expect(summariseQuery('SELECT\n  1,\n  2')).toBe('SELECT 1, 2')
  })

  it('cuts a long statement', () => {
    expect(summariseQuery('SELECT '.repeat(40), 10)).toHaveLength(11)
  })
})
