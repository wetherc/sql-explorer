import { describe, expect, it } from 'vitest'
import { ResultStream, type ResultTable, type RunEnd } from '@/lib/results'
import type { Message } from '@/types/api'
import { runFrames, type SampleSet } from '../frames'

/** Reads the frames with the reader of the application. */
function read(buffer: ArrayBuffer) {
  const sets: ResultTable[] = []
  const messages: Message[] = []
  let end: RunEnd | null = null
  const stream = new ResultStream({
    onSet: (table) => sets.push(table),
    onMessage: (message) => messages.push(message),
    onEnd: (value) => (end = value),
  })
  stream.feed(buffer)
  expect(stream.failure).toBeNull()
  return { sets, messages, end }
}

function rows(table: ResultTable): unknown[][] {
  return Array.from({ length: table.rowCount }, (_, row) =>
    table.columns.map((_, column) => table.cell(row, column)),
  )
}

const END: RunEnd = { messages: [], rowsAffected: null, elapsedMs: 12, stats: null }

describe('runFrames', () => {
  it('writes every encoding so the reader of the application gives the values back', () => {
    const set: SampleSet = {
      columns: [
        { name: 'none', typeName: 'text' },
        { name: 'flag', typeName: 'boolean' },
        { name: 'small', typeName: 'integer' },
        { name: 'real', typeName: 'double' },
        { name: 'big', typeName: 'bigint' },
        { name: 'name', typeName: 'text' },
        { name: 'doc', typeName: 'json' },
      ],
      rows: [
        [null, true, -7, 1.5, 2 ** 40, 'Ada', { a: 1 }],
        [null, null, null, null, null, null, null],
        [null, false, 2_147_483_647, -0.25, 3, 'Grace Hopper', [1, 'two']],
        [null, true, 0, 0, 0, '', 'text in json'],
        [null, false, 1, 2, 3, 'é', { b: null }],
        [null, true, 1, 2, 3, 'x', { c: true }],
        [null, false, 1, 2, 3, 'y', { d: 'e' }],
        [null, true, 1, 2, 3, 'z', { f: 1.5 }],
        [null, false, 1, 2, 3, 'nine rows', { g: [] }],
      ],
    }
    const { sets, end } = read(runFrames([set], [], END))
    expect(sets).toHaveLength(1)
    expect(sets[0]!.columns).toEqual(set.columns)
    expect(sets[0]!.truncated).toBe(false)
    const back = rows(sets[0]!)
    expect(back[0]).toEqual([null, true, -7, 1.5, 2 ** 40, 'Ada', { a: 1 }])
    expect(back[1]).toEqual([null, null, null, null, null, null, null])
    expect(back[2]!.slice(0, 6)).toEqual([null, false, 2_147_483_647, -0.25, 3, 'Grace Hopper'])
    expect(back[4]![5]).toBe('é')
    expect(back[8]![5]).toBe('nine rows')
    expect(end).toEqual(END)
  })

  it('numbers the sets in order and writes the messages and the truncated mark', () => {
    const first: SampleSet = { columns: [{ name: 'n', typeName: 'int' }], rows: [[1]] }
    const second: SampleSet = {
      columns: [{ name: 's', typeName: 'text' }],
      rows: [['a'], ['b']],
      truncated: true,
    }
    const message: Message = { level: 'info', text: 'done', detail: null }
    const { sets, messages } = read(runFrames([first, second], [message], END))
    expect(sets.map(rows)).toEqual([[[1]], [['a'], ['b']]])
    expect(sets.map((table) => table.truncated)).toEqual([false, true])
    expect(messages).toEqual([message])
  })

  it('reads a missing cell of a short row as null', () => {
    const set: SampleSet = {
      columns: [
        { name: 'a', typeName: 'int' },
        { name: 'b', typeName: 'int' },
      ],
      rows: [[1]],
    }
    const { sets } = read(runFrames([set], [], END))
    expect(rows(sets[0]!)).toEqual([[1, null]])
  })
})
