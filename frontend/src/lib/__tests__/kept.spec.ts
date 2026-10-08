import { describe, expect, it } from 'vitest'
import { exportAllNote, keptNote, shortCount, shortSize, timeAgo, unsavedNote } from '@/lib/kept'

describe('kept result notes', () => {
  it('writes a size with few digits', () => {
    expect(shortSize(0)).toBe('0 B')
    expect(shortSize(1023)).toBe('1023 B')
    expect(shortSize(1536)).toBe('1.5 KB')
    expect(shortSize(412 * 1024 ** 2)).toBe('412 MB')
    expect(shortSize(1.2 * 1024 ** 3)).toBe('1.2 GB')
    expect(shortSize(3 * 1024 ** 5)).toBe('3072 TB')
  })

  it('writes a large count with few digits', () => {
    expect(shortCount(950)).toBe('950')
    expect(shortCount(1_234_567)).toBe('1.2M')
  })

  it('writes the age of a moment', () => {
    expect(timeAgo(1000, 1000)).toBe('just now')
    expect(timeAgo(5000, 1000)).toBe('just now')
    expect(timeAgo(0, 59 * 60_000)).toBe('59 min ago')
    expect(timeAgo(0, 3 * 3_600_000)).toBe('3 h ago')
  })

  it('says where the full rows are', () => {
    const now = 2 * 60_000
    expect(keptNote({ origin: 'athena', keptAt: 0 }, now)).toBe('Saved on Athena, 2 min ago')
    expect(keptNote({ origin: 'paused', keptAt: now }, now)).toBe('Paused on the server, just now')
    expect(keptNote({ origin: 'spill', keptAt: 0 }, now)).toBe(
      'Saved on this computer, 0 rows, 0 B',
    )
  })

  it('says where an export of all rows takes its rows from', () => {
    expect(exportAllNote({ origin: 'paused', keptAt: 0 }, 5)).toBe('Continues the paused read')
    expect(exportAllNote(null, null)).toBe('Runs the query again')
    expect(exportAllNote(null, 1500)).toBe('Runs the query again (last run took 1.50 s)')
  })

  it('says why the full rows were not saved', () => {
    expect(
      (
        ['script', 'diskLimit', 'exportLimit', 'stopped', 'diskFailed', 'stoppedSaving'] as const
      ).map(unsavedNote),
    ).toEqual([
      "Not saved: scripts with more than one statement can't be saved",
      'Not saved: the disk limit was reached',
      'Not saved: the export row limit was reached',
      'Not saved: the read stopped',
      "Not saved: the file couldn't be written on this computer",
      'Not saved: you stopped saving',
    ])
  })
})
