import { describe, expect, it } from 'vitest'
import {
  BYTES_IN_TERABYTE,
  NULL_TEXT,
  formatBytes,
  formatCost,
  scanCost,
  formatClockTime,
  compareCells,
  compareSortKeys,
  sortKey,
  formatCell,
  formatDuration,
  formatRowCount,
  stoppedStatementsMessage,
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

describe('sortKey', () => {
  it('gives no key for a cell without a value', () => {
    expect(sortKey(null)).toBeNull()
  })

  it('keeps a number and writes every other cell as text', () => {
    expect(sortKey(7)).toBe(7)
    expect(sortKey(true)).toBe('true')
    expect(sortKey('Ada')).toBe('Ada')
  })

  it('compares a number and a text that holds a number by value', () => {
    expect(compareSortKeys(sortKey(2), sortKey('10'))).toBeLessThan(0)
    expect(compareSortKeys(sortKey('10'), sortKey(2))).toBeGreaterThan(0)
    expect(compareSortKeys(sortKey(-0.05), sortKey('-5e-2'))).toBe(0)
  })

  it('compares texts that hold decimal numbers by value', () => {
    const texts = ['1.5', '-10.00', '1.25', '0', '-5.00', '.5', '-0.0', '1e3', '999', '0.05', '+2']
    const sorted = [...texts].sort((left, right) => compareSortKeys(sortKey(left), sortKey(right)))
    expect(sorted).toEqual([
      '-10.00',
      '-5.00',
      '0',
      '-0.0',
      '0.05',
      '.5',
      '1.25',
      '1.5',
      '+2',
      '999',
      '1e3',
    ])
    // A decimal key keeps every digit.
    expect(
      compareSortKeys(sortKey('12345678901234567.1'), sortKey('12345678901234567.2')),
    ).toBeLessThan(0)
    expect(compareSortKeys(sortKey('-1.5'), sortKey('-1.25'))).toBeLessThan(0)
    expect(compareSortKeys(sortKey('2'), sortKey('2.0'))).toBe(0)
  })

  it('compares a decimal text with other text and with an infinite number as text', () => {
    expect(compareSortKeys(sortKey('5'), sortKey('abc'))).toBeLessThan(0)
    expect(compareSortKeys(sortKey(Infinity), sortKey('5'))).toBeGreaterThan(0)
    // A text of another cell kind stays text.
    expect(sortKey([1])).toBe('[1]')
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

describe('formatClockTime', () => {
  it('writes the time of day of a moment', () => {
    const moment = new Date(2026, 0, 2, 13, 45, 7).getTime()
    expect(formatClockTime(moment)).toBe(new Date(moment).toLocaleTimeString())
  })
})

describe('formatBytes', () => {
  it('uses the largest unit that keeps the number above one', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(2048)).toBe('2.00 KB')
    expect(formatBytes(5 * 1024 ** 2)).toBe('5.00 MB')
    expect(formatBytes(3 * 1024 ** 3)).toBe('3.00 GB')
    expect(formatBytes(4 * BYTES_IN_TERABYTE)).toBe('4.00 TB')
    expect(formatBytes(4096 * BYTES_IN_TERABYTE)).toBe('4096.00 TB')
  })
})

describe('scanCost', () => {
  it('gives the price of a scan from the rate for each terabyte', () => {
    expect(scanCost(BYTES_IN_TERABYTE, 5)).toBe(5)
    expect(scanCost(BYTES_IN_TERABYTE / 2, 5)).toBe(2.5)
    expect(scanCost(0, 5)).toBe(0)
  })
})

describe('formatCost', () => {
  it('writes a price in dollars', () => {
    expect(formatCost(0)).toBe('$0.00')
    expect(formatCost(2.5)).toBe('$2.50')
  })

  it('keeps four places for a price below one cent', () => {
    expect(formatCost(0.0004)).toBe('$0.0004')
  })
})

describe('stoppedStatementsMessage', () => {
  it('names one statement in the singular', () => {
    expect(stoppedStatementsMessage(1)).toContain('One statement is running')
  })

  it('counts more than one', () => {
    expect(stoppedStatementsMessage(3)).toContain('3 statements are running')
  })

  it('says what the close costs', () => {
    expect(stoppedStatementsMessage(2)).toContain('their rows are lost')
  })
})
