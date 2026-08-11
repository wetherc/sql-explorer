import { describe, expect, it, vi } from 'vitest'
import { ResultStream, ResultTable, type RunEnd } from '../results'
import type { CellValue, ColumnInfo } from '@/types/api'

const FRAME_BEGIN_SET = 1
const FRAME_CHUNK = 2
const FRAME_END_SET = 3
const FRAME_END = 4

/**
 * A writer of the frames, which holds the same form as the writer of the
 * backend in `backend/src/db/columnar.rs`.
 */
class Writer {
  private bytes: number[] = []

  u8(value: number): this {
    this.bytes.push(value & 0xff)
    return this
  }

  u32(value: number): this {
    this.bytes.push(value & 0xff, (value >> 8) & 0xff, (value >> 16) & 0xff, (value >>> 24) & 0xff)
    return this
  }

  i32(value: number): this {
    return this.u32(value >>> 0)
  }

  f64(value: number): this {
    const buffer = new ArrayBuffer(8)
    new DataView(buffer).setFloat64(0, value, true)
    this.raw([...new Uint8Array(buffer)])
    return this
  }

  raw(values: number[]): this {
    this.bytes.push(...values)
    return this
  }

  text(value: string): this {
    const bytes = new TextEncoder().encode(value)
    this.u32(bytes.length)
    return this.raw([...bytes])
  }

  pad(width: number): this {
    while (this.bytes.length % width !== 0) {
      this.bytes.push(0)
    }
    return this
  }

  buffer(): ArrayBuffer {
    return new Uint8Array(this.bytes).buffer
  }
}

/** The frames of one set of one column of whole numbers. */
function intSetMessage(values: Array<number | null>): ArrayBuffer {
  const writer = new Writer()
  writer.u8(FRAME_BEGIN_SET).u32(0).u32(1).text('n').text('int')
  writer.u8(FRAME_CHUNK).u32(0).u32(values.length).u32(1)
  writer.u8(2)
  let mask = 0
  values.forEach((value, index) => {
    if (value === null) {
      mask |= 1 << index
    }
  })
  writer.u8(mask).pad(4)
  for (const value of values) {
    writer.i32(value ?? 0)
  }
  writer.u8(FRAME_END_SET).u32(0).u8(0)
  return writer.buffer()
}

/**
 * The frames of one set of one column of text that holds each of its texts
 * once, as the backend writes it for a column whose values repeat.
 */
function dictSetMessage(values: Array<string | null>): ArrayBuffer {
  const writer = new Writer()
  writer.u8(FRAME_BEGIN_SET).u32(0).u32(1).text('state').text('text')
  writer.u8(FRAME_CHUNK).u32(0).u32(values.length).u32(1)
  writer.u8(6)
  const mask = new Uint8Array(Math.ceil(values.length / 8))
  values.forEach((value, index) => {
    if (value === null) {
      mask[index >> 3]! |= 1 << (index % 8)
    }
  })
  writer.raw([...mask])
  writer.pad(4)
  const texts: string[] = []
  const codes: number[] = []
  for (const value of values) {
    if (value === null) {
      codes.push(0)
      continue
    }
    let code = texts.indexOf(value)
    if (code < 0) {
      code = texts.length
      texts.push(value)
    }
    codes.push(code)
  }
  const bytes = new TextEncoder().encode(texts.join(''))
  writer.u32(texts.length)
  let end = 0
  for (const text of texts) {
    end += new TextEncoder().encode(text).length
    writer.u32(end)
  }
  writer.u32(bytes.length).raw([...bytes])
  writer.pad(4)
  for (const code of codes) {
    writer.u32(code)
  }
  writer.u8(FRAME_END_SET).u32(0).u8(0)
  return writer.buffer()
}

function collect(): {
  stream: ResultStream
  sets: ResultTable[]
  ends: RunEnd[]
} {
  const sets: ResultTable[] = []
  const ends: RunEnd[] = []
  const stream = new ResultStream({
    onSet: (table) => sets.push(table),
    onEnd: (end) => ends.push(end),
  })
  return { stream, sets, ends }
}

describe('ResultTable from plain rows', () => {
  const columns: ColumnInfo[] = [
    { name: 'id', typeName: 'int' },
    { name: 'name', typeName: 'text' },
  ]

  it('gives the cells, the rows and a window of the rows', () => {
    const table = ResultTable.fromRows(columns, [
      [1, 'one'],
      [2, null],
    ])
    expect(table.rowCount).toBe(2)
    expect(table.columns).toEqual(columns)
    expect(table.cell(0, 1)).toBe('one')
    expect(table.cell(1, 1)).toBeNull()
    expect(table.row(1)).toEqual([2, null])
    expect(table.slice(0, 5)).toEqual([
      [1, 'one'],
      [2, null],
    ])
    expect([...table.rows()]).toHaveLength(2)
  })

  it('holds no row and no place past its end', () => {
    const table = ResultTable.fromRows(columns, [])
    expect(table.rowCount).toBe(0)
    expect(table.cell(0, 0)).toBeNull()
    expect(table.slice(0, 10)).toEqual([])
    const one = ResultTable.fromRows(columns, [[1, 'one']])
    expect(one.cell(4, 0)).toBeNull()
    expect(one.cell(0, 7)).toBeNull()
  })

  it('keeps the mark of a read that a limit stopped', () => {
    expect(ResultTable.fromRows(columns, [[1, 'a']], true).truncated).toBe(true)
  })
})

describe('the reader of the chunks', () => {
  it('reads a set of whole numbers with its nulls', () => {
    const { stream, sets } = collect()
    stream.feed(intSetMessage([1, null, -2]))

    expect(sets).toHaveLength(1)
    const table = sets[0]!
    expect(table.columns).toEqual([{ name: 'n', typeName: 'int' }])
    expect(table.rowCount).toBe(3)
    expect(table.slice(0, 3)).toEqual([[1], [null], [-2]])
    expect(table.truncated).toBe(false)
  })

  it('reads the values of every form', () => {
    const writer = new Writer()
    writer
      .u8(FRAME_BEGIN_SET)
      .u32(0)
      .u32(6)
      .text('nothing')
      .text('null')
      .text('flag')
      .text('bit')
      .text('count')
      .text('int')
      .text('size')
      .text('float')
      .text('name')
      .text('text')
      .text('doc')
      .text('json')
    writer.u8(FRAME_CHUNK).u32(0).u32(2).u32(6)
    // Nothing holds a value.
    writer.u8(0)
    // Two flags, the second one null.
    writer.u8(1).u8(0b10).u8(0b01)
    // Two whole numbers.
    writer.u8(2).u8(0).pad(4).i32(7).i32(-8)
    // Two doubles.
    writer.u8(3).u8(0).pad(8).f64(1.5).f64(2.25)
    // Two texts, the second one null.
    writer
      .u8(4)
      .u8(0b10)
      .pad(4)
      .u32(3)
      .u32(3)
      .u32(3)
      .raw([...new TextEncoder().encode('one')])
    // Two values of JSON.
    const json = new TextEncoder().encode('{"a":1}[1,2]')
    writer
      .u8(5)
      .u8(0)
      .pad(4)
      .u32(7)
      .u32(12)
      .u32(json.length)
      .raw([...json])
    writer.u8(FRAME_END_SET).u32(0).u8(1)

    const { stream, sets } = collect()
    stream.feed(writer.buffer())

    const table = sets[0]!
    expect(table.truncated).toBe(true)
    expect(table.row(0)).toEqual([null, true, 7, 1.5, 'one', { a: 1 }])
    expect(table.row(1)).toEqual([null, null, -8, 2.25, null, [1, 2]])
    // A second read of a text gives the value that the first read kept.
    expect(table.cell(0, 4)).toBe('one')
  })

  it('joins the chunks of one set into one table', () => {
    const first = new Writer()
    first.u8(FRAME_BEGIN_SET).u32(0).u32(1).text('n').text('int')
    first.u8(FRAME_CHUNK).u32(0).u32(2).u32(1).u8(2).u8(0).pad(4).i32(1).i32(2)
    const second = new Writer()
    second.u8(FRAME_CHUNK).u32(0).u32(1).u32(1).u8(2).u8(0).pad(4).i32(3)
    second.u8(FRAME_END_SET).u32(0).u8(0)

    const { stream, sets } = collect()
    stream.feed(first.buffer())
    expect(sets).toHaveLength(0)
    stream.feed(second.buffer())

    const table = sets[0]!
    expect(table.rowCount).toBe(3)
    expect(table.slice(0, 3)).toEqual([[1], [2], [3]])
    // A read that goes back holds the segment of the earlier chunk again.
    expect(table.cell(0, 0)).toBe(1)
  })

  it('keeps the sets of a script apart', () => {
    const writer = new Writer()
    writer.u8(FRAME_BEGIN_SET).u32(0).u32(1).text('a').text('int')
    writer.u8(FRAME_CHUNK).u32(0).u32(1).u32(1).u8(2).u8(0).pad(4).i32(1)
    writer.u8(FRAME_END_SET).u32(0).u8(0)
    writer.u8(FRAME_BEGIN_SET).u32(1).u32(1).text('b').text('int')
    writer.u8(FRAME_CHUNK).u32(1).u32(1).u32(1).u8(2).u8(0).pad(4).i32(2)
    writer.u8(FRAME_END_SET).u32(1).u8(0)

    const { stream, sets } = collect()
    stream.feed(writer.buffer())

    expect(sets.map((table) => table.columns[0]?.name)).toEqual(['a', 'b'])
    expect(sets[1]!.slice(0, 1)).toEqual([[2]])
  })

  it('reports what the run ended with', () => {
    const writer = new Writer()
    writer.u8(FRAME_END).text(
      JSON.stringify({
        messages: [{ level: 'info', text: '1 row returned.', detail: null }],
        rowsAffected: 3,
        elapsedMs: 8,
        stats: null,
      }),
    )

    const { stream, ends } = collect()
    stream.feed(writer.buffer())

    expect(ends[0]?.rowsAffected).toBe(3)
    expect(ends[0]?.elapsedMs).toBe(8)
    expect(ends[0]?.messages[0]?.text).toBe('1 row returned.')
  })

  it('refuses a frame and a form that it does not know', () => {
    const { stream } = collect()
    expect(() => stream.feed(new Uint8Array([99]).buffer)).toThrow(/frame of the unknown kind 99/)

    const writer = new Writer()
    writer.u8(FRAME_BEGIN_SET).u32(0).u32(1).text('n').text('int')
    writer.u8(FRAME_CHUNK).u32(0).u32(1).u32(1).u8(9)
    expect(() => stream.feed(writer.buffer())).toThrow(/column of the unknown form 9/)
  })

  it('holds no set that it never opened', () => {
    const writer = new Writer()
    writer.u8(FRAME_CHUNK).u32(4).u32(1).u32(1).u8(2).u8(0).pad(4).i32(1)
    writer.u8(FRAME_END_SET).u32(4).u8(0)
    const { stream, sets } = collect()
    stream.feed(writer.buffer())
    expect(sets).toHaveLength(0)
  })

  it('reads the bytes that the backend writes for a small chunk', () => {
    // The backend holds the same case, so the two sides keep one form.
    const bytes = new Uint8Array([
      FRAME_CHUNK,
      0,
      0,
      0,
      0, // the set
      2,
      0,
      0,
      0, // the number of rows
      1,
      0,
      0,
      0, // the number of columns
      2, // the whole numbers
      0, // the mask of the nulls
      0, // the padding to four bytes
      1, // the first value
      0,
      0,
      0,
      0xfe, // the second value
      0xff,
      0xff,
      0xff,
    ])
    const { stream, sets } = collect()
    const opening = new Writer()
    opening.u8(FRAME_BEGIN_SET).u32(0).u32(1).text('n').text('int')
    stream.feed(opening.buffer())
    stream.feed(bytes.buffer)
    const closing = new Writer()
    closing.u8(FRAME_END_SET).u32(0).u8(0)
    stream.feed(closing.buffer())

    expect(sets[0]!.slice(0, 2)).toEqual([[1], [-2]])
  })

  it('holds one row at a time while it walks the rows', () => {
    const { stream, sets } = collect()
    stream.feed(intSetMessage([1, 2, 3]))
    const walked: CellValue[][] = []
    for (const row of sets[0]!.rows()) {
      walked.push(row)
    }
    expect(walked).toEqual([[1], [2], [3]])
  })

  it('reads a column of texts that hold each text once', () => {
    const { stream, sets } = collect()
    stream.feed(dictSetMessage(['open', 'shut', 'open', null, 'shut']))

    const table = sets[0]!
    expect(table.rowCount).toBe(5)
    expect(table.slice(0, 5)).toEqual([['open'], ['shut'], ['open'], [null], ['shut']])
    // A second read gives the text that the first read kept.
    expect(table.cell(2, 0)).toBe('open')
  })

  it('builds one text for each different text of a column', () => {
    const values = Array.from({ length: 300 }, (_, index) => `state ${index % 3}`)
    const { stream, sets } = collect()
    stream.feed(dictSetMessage(values))

    const table = sets[0]!
    const decode = vi.spyOn(TextDecoder.prototype, 'decode')
    for (const row of table.rows()) {
      expect(typeof row[0]).toBe('string')
    }
    const built = decode.mock.calls.length
    decode.mockRestore()
    expect(built).toBe(3)
  })

  it('refuses a code that names no text of the dictionary', () => {
    const writer = new Writer()
    writer.u8(FRAME_BEGIN_SET).u32(0).u32(1).text('state').text('text')
    writer.u8(FRAME_CHUNK).u32(0).u32(2).u32(1)
    writer.u8(6)
    writer.u8(0).pad(4)
    const bytes = [...new TextEncoder().encode('open')]
    writer.u32(1).u32(bytes.length).u32(bytes.length).raw(bytes)
    writer.pad(4)
    writer.u32(0).u32(5)
    writer.u8(FRAME_END_SET).u32(0).u8(0)

    const { stream, sets } = collect()
    stream.feed(writer.buffer())
    const table = sets[0]!
    expect(table.cell(0, 0)).toBe('open')
    expect(() => table.cell(1, 0)).toThrow(/name the text 5 of a dictionary of 1/)
  })

  it('gives no value for a column that the chunk does not hold', () => {
    const { stream, sets } = collect()
    stream.feed(intSetMessage([1]))
    expect(sets[0]!.cell(0, 3)).toBeNull()
  })
})

describe('the cost of a large result', () => {
  it('reads a chunk of ten thousand rows without a row of objects', () => {
    const rows = 10_000
    const writer = new Writer()
    writer.u8(FRAME_BEGIN_SET).u32(0).u32(1).text('n').text('int')
    writer.u8(FRAME_CHUNK).u32(0).u32(rows).u32(1).u8(2)
    for (let index = 0; index < Math.ceil(rows / 8); index += 1) {
      writer.u8(0)
    }
    writer.pad(4)
    for (let index = 0; index < rows; index += 1) {
      writer.i32(index)
    }
    writer.u8(FRAME_END_SET).u32(0).u8(0)

    const { stream, sets } = collect()
    const build = vi.spyOn(Array.prototype, 'map')
    stream.feed(writer.buffer())
    build.mockRestore()

    const table = sets[0]!
    expect(table.rowCount).toBe(rows)
    expect(table.cell(9999, 0)).toBe(9999)
    // The window of a grid builds the rows it draws and no others.
    expect(table.slice(9990, 10_000)).toHaveLength(10)
  })
})
