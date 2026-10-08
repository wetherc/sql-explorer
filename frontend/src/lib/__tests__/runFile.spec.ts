import { afterEach, describe, expect, it } from 'vitest'
import {
  SET_CHOICE_KEY,
  SHEET_ROW_ROOM,
  csvFileName,
  eachSetLabel,
  excelRowsOver,
  loadSetChoice,
  saveSetChoice,
  savedFile,
  savedFileNote,
  savedSetsMessage,
  secondFileName,
} from '@/lib/runFile'
import type { RunFileSummary, SavedSet } from '@/types/api'

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

  it('says when the Excel sheet had no room for more rows', () => {
    const file = { path: '/a/out.xlsx', rows: 1048575, truncated: true, sheetFull: true }
    expect(savedFileNote(10, true, file)).toBe(
      'Showing the first 10 rows. The first 1,048,575 rows were saved to /a/out.xlsx. The Excel sheet has no room for more rows.',
    )
  })

  it('names the sheet of a result in an Excel file with a sheet for each result', () => {
    const file = { path: '/a/out.xlsx', rows: 2, truncated: false, sheet: 'Result 2' }
    expect(savedFileNote(2, false, file)).toBe(
      'All 2 rows were saved to the "Result 2" sheet of /a/out.xlsx.',
    )
  })
})

const set = (sheet: string | null): SavedSet => ({
  path: '/a/out.xlsx',
  sheet,
  rows: 4,
  truncated: true,
  sheetFull: false,
})

describe('savedFile', () => {
  it('keeps the sheet of a set only when it has one', () => {
    expect(savedFile(set(null))).toEqual({ path: '/a/out.xlsx', rows: 4, truncated: true })
    expect(savedFile(set('Result 1')).sheet).toBe('Result 1')
  })

  it('marks a set whose sheet was full', () => {
    expect(savedFile({ ...set(null), sheetFull: true }).sheetFull).toBe(true)
  })
})

describe('the room of an Excel sheet', () => {
  it('flags a row limit past the room of a sheet for an Excel file alone', () => {
    expect(excelRowsOver('xlsx', SHEET_ROW_ROOM)).toBe(false)
    expect(excelRowsOver('xlsx', SHEET_ROW_ROOM + 1)).toBe(true)
    expect(excelRowsOver('csv', SHEET_ROW_ROOM + 1)).toBe(false)
  })

  it('names a CSV file in place of the chosen file', () => {
    expect(csvFileName('/a/orders.xlsx')).toBe('orders.csv')
    expect(csvFileName('C:\\a\\plain')).toBe('plain.csv')
  })
})

describe('the choice of the result sets', () => {
  afterEach(() => {
    localStorage.clear()
  })

  it('remembers the last choice', () => {
    expect(loadSetChoice()).toBe('first')
    saveSetChoice('each')
    expect(localStorage.getItem(SET_CHOICE_KEY)).toBe('each')
    expect(loadSetChoice()).toBe('each')
  })

  it('falls back on the first result when the store refuses', () => {
    localStorage.setItem(SET_CHOICE_KEY, 'each')
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      get() {
        throw new Error('blocked')
      },
    })
    try {
      expect(() => saveSetChoice('each')).not.toThrow()
      expect(loadSetChoice()).toBe('first')
    } finally {
      Object.defineProperty(globalThis, 'localStorage', descriptor!)
    }
  })

  it('names the choice for the format', () => {
    expect(eachSetLabel('xlsx')).toBe('One sheet per result set')
    expect(eachSetLabel('json')).toBe('One file per result set')
  })
})

describe('secondFileName', () => {
  it('puts the number before the extension', () => {
    expect(secondFileName('/a/orders.csv')).toBe('orders-2.csv')
    expect(secondFileName('C:\\a\\b.c.json')).toBe('b.c-2.json')
    expect(secondFileName('/a/plain')).toBe('plain-2')
    expect(secondFileName('/a/.hidden')).toBe('.hidden-2')
  })
})

describe('savedSetsMessage', () => {
  const summary = (sets: SavedSet[]): RunFileSummary => ({
    rows: 8,
    truncated: false,
    path: '/a/out.xlsx',
    sheetFull: false,
    cutCells: 0,
    warning: null,
    sets,
    skippedSets: 0,
  })

  it('has no words for one saved set', () => {
    expect(savedSetsMessage(summary([set(null)]))).toBeNull()
  })

  it('counts the sheets or the files', () => {
    expect(savedSetsMessage(summary([set('Result 1'), set('Result 2')]))).toBe(
      'Saved 8 rows to 2 sheets in /a/out.xlsx.',
    )
    expect(savedSetsMessage(summary([set(null), set(null), set(null)]))).toBe(
      'Saved 8 rows to 3 files, starting with /a/out.xlsx.',
    )
  })
})
