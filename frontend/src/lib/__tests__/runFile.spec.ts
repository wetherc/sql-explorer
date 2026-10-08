import { describe, expect, it } from 'vitest'
import { savedFileNote } from '@/lib/runFile'

describe('savedFileNote', () => {
  it('names the preview and the whole file', () => {
    expect(savedFileNote(1000, true, { path: '/a/out.csv', rows: 52310, truncated: false })).toBe(
      'Showing the first 1,000 rows. All 52,310 rows were saved to /a/out.csv.',
    )
  })

  it('says when the file stopped at its limit', () => {
    expect(savedFileNote(10, true, { path: '/a/out.csv', rows: 20, truncated: true })).toBe(
      'Showing the first 10 rows. The first 20 rows were saved to /a/out.csv.',
    )
    expect(savedFileNote(1, true, { path: '/a/out.csv', rows: 1, truncated: true })).toBe(
      'Showing the first 1 row. The first 1 row was saved to /a/out.csv.',
    )
  })

  it('names only the file when the grid shows every row', () => {
    expect(savedFileNote(3, false, { path: '/a/out.json', rows: 3, truncated: false })).toBe(
      'All 3 rows were saved to /a/out.json.',
    )
    expect(savedFileNote(1, false, { path: '/a/out.json', rows: 1, truncated: false })).toBe(
      'The row was saved to /a/out.json.',
    )
  })
})
