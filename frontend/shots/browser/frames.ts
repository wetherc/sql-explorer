/**
 * Writes the binary frames of one run, in the form that the backend writes in
 * `backend/src/db/columnar.rs` and that `ResultStream` in `src/lib/results.ts`
 * reads. The screenshot backend sends its sample rows through the same reader
 * as a real run, so the grid shows them as it shows rows of a server.
 */
import type { RunEnd } from '@/lib/results'
import type { CellValue, ColumnInfo, Message } from '@/types/api'

const FRAME_BEGIN_SET = 1
const FRAME_CHUNK = 2
const FRAME_END_SET = 3
const FRAME_END = 4
const FRAME_MESSAGE = 5

const ENCODING_NULL = 0
const ENCODING_BOOL = 1
const ENCODING_INT32 = 2
const ENCODING_FLOAT64 = 3
const ENCODING_TEXT = 4
const ENCODING_JSON = 5

const encoder = new TextEncoder()

/** One result set of a sample run. */
export interface SampleSet {
  columns: ColumnInfo[]
  rows: CellValue[][]
  truncated?: boolean
}

/** Appends bytes in little-endian order and keeps the offsets aligned. */
class Writer {
  private readonly bytes: number[] = []

  u8(value: number): this {
    this.bytes.push(value & 0xff)
    return this
  }

  u32(value: number): this {
    this.bytes.push(value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, value >>> 24)
    return this
  }

  f64(value: number): this {
    const view = new DataView(new ArrayBuffer(8))
    view.setFloat64(0, value, true)
    return this.raw(new Uint8Array(view.buffer))
  }

  raw(values: Uint8Array): this {
    for (const value of values) {
      this.bytes.push(value)
    }
    return this
  }

  text(value: string): this {
    const bytes = encoder.encode(value)
    return this.u32(bytes.length).raw(bytes)
  }

  /** Pads with zeros until the length is a multiple of the width. */
  align(width: number): this {
    while (this.bytes.length % width !== 0) {
      this.bytes.push(0)
    }
    return this
  }

  buffer(): ArrayBuffer {
    return new Uint8Array(this.bytes).buffer
  }
}

/** The encoding that fits every value of one column. */
function encodingOf(values: CellValue[]): number {
  const present = values.filter((value) => value !== null)
  if (present.length === 0) {
    return ENCODING_NULL
  }
  if (present.every((value) => typeof value === 'boolean')) {
    return ENCODING_BOOL
  }
  if (present.every((value) => Number.isInteger(value) && Math.abs(value as number) < 2 ** 31)) {
    return ENCODING_INT32
  }
  if (present.every((value) => typeof value === 'number')) {
    return ENCODING_FLOAT64
  }
  if (present.every((value) => typeof value === 'string')) {
    return ENCODING_TEXT
  }
  return ENCODING_JSON
}

/** A mask with one bit for each row, set where `test` holds. */
function mask(values: CellValue[], test: (value: CellValue) => boolean): Uint8Array {
  const bits = new Uint8Array(Math.ceil(values.length / 8))
  values.forEach((value, row) => {
    if (test(value)) {
      bits[row >> 3]! |= 1 << (row % 8)
    }
  })
  return bits
}

function writeColumn(writer: Writer, values: CellValue[]): void {
  const encoding = encodingOf(values)
  writer.u8(encoding)
  if (encoding === ENCODING_NULL) {
    return
  }
  writer.raw(mask(values, (value) => value === null))
  switch (encoding) {
    case ENCODING_BOOL:
      writer.raw(mask(values, (value) => value === true))
      return
    case ENCODING_INT32:
      writer.align(4)
      for (const value of values) {
        writer.u32((value as number | null) ?? 0)
      }
      return
    case ENCODING_FLOAT64:
      writer.align(8)
      for (const value of values) {
        writer.f64((value as number | null) ?? 0)
      }
      return
    default: {
      const texts = values.map((value) => {
        if (value === null) {
          return new Uint8Array()
        }
        return encoder.encode(
          encoding === ENCODING_TEXT ? (value as string) : JSON.stringify(value),
        )
      })
      writer.align(4)
      let end = 0
      for (const text of texts) {
        end += text.length
        writer.u32(end)
      }
      writer.u32(end)
      for (const text of texts) {
        writer.raw(text)
      }
    }
  }
}

/**
 * The frames of one whole run: each set in one chunk, the messages, and the
 * frame that ends the run.
 */
export function runFrames(sets: SampleSet[], messages: Message[], end: RunEnd): ArrayBuffer {
  const writer = new Writer()
  sets.forEach((set, index) => {
    writer.u8(FRAME_BEGIN_SET).u32(index).u32(set.columns.length)
    for (const column of set.columns) {
      writer.text(column.name).text(column.typeName)
    }
    writer.u8(FRAME_CHUNK).u32(index).u32(set.rows.length).u32(set.columns.length)
    set.columns.forEach((_, column) => {
      writeColumn(
        writer,
        set.rows.map((row) => row[column] ?? null),
      )
    })
    writer
      .u8(FRAME_END_SET)
      .u32(index)
      .u8(set.truncated ? 1 : 0)
  })
  for (const message of messages) {
    writer.u8(FRAME_MESSAGE).text(JSON.stringify(message))
  }
  writer.u8(FRAME_END).text(JSON.stringify(end))
  return writer.buffer()
}
