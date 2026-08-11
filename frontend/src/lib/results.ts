/**
 * The rows of one execution, held as the bytes that the backend sent.
 *
 * The backend sends the rows in chunks, and each chunk holds its values column
 * by column. This module keeps the buffer of each chunk and reads a value out
 * of it when the interface asks for one. A result of many rows therefore costs
 * no object for a row and no object for a cell.
 *
 * The form of the bytes stands in `backend/src/db/columnar.rs`.
 */

import type { CellValue, ColumnInfo, Message, QueryStats } from '@/types/api'

const FRAME_BEGIN_SET = 1
const FRAME_CHUNK = 2
const FRAME_END_SET = 3
const FRAME_END = 4

const ENCODING_NULL = 0
const ENCODING_BOOL = 1
const ENCODING_INT32 = 2
const ENCODING_FLOAT64 = 3
const ENCODING_TEXT = 4
const ENCODING_JSON = 5

const decoder = new TextDecoder()

/** One column of one chunk, in the form the bytes carry. */
type SegmentColumn =
  | { kind: 'null' }
  | { kind: 'bool'; nulls: Uint8Array; values: Uint8Array }
  | { kind: 'int32'; nulls: Uint8Array; values: Int32Array }
  | { kind: 'float64'; nulls: Uint8Array; values: Float64Array }
  | {
      kind: 'text' | 'json'
      nulls: Uint8Array
      ends: Uint32Array
      bytes: Uint8Array
      /** The values that were read already, so a filter reads each once. */
      cache: Array<CellValue | undefined>
    }

/**
 * One chunk of rows, with the place of its first row in the result. A table
 * that a caller built from plain rows holds those rows in `plain`.
 */
interface Segment {
  start: number
  length: number
  columns: SegmentColumn[]
  plain?: CellValue[][]
}

/** True when the bit of one row is set in a mask of bits. */
function bitSet(mask: Uint8Array, row: number): boolean {
  const byte = mask[row >> 3] ?? 0
  return (byte & (1 << (row % 8))) !== 0
}

/**
 * The rows of one result set. The interface reads a cell, a row or a window of
 * rows, and the table reads those out of the bytes it holds.
 */
export class ResultTable {
  readonly columns: ColumnInfo[]
  truncated = false
  private readonly segments: Segment[] = []
  private rows_ = 0
  /** The segment of the last read, from which most reads go on. */
  private lastSegment = 0

  constructor(columns: ColumnInfo[]) {
    this.columns = columns
  }

  /** Builds a table from plain rows, for a plan of a statement and for tests. */
  static fromRows(columns: ColumnInfo[], rows: CellValue[][], truncated = false): ResultTable {
    const table = new ResultTable(columns)
    table.truncated = truncated
    if (rows.length > 0) {
      table.segments.push({ start: 0, length: rows.length, columns: [], plain: rows })
      table.rows_ = rows.length
    }
    return table
  }

  get rowCount(): number {
    return this.rows_
  }

  /** Adds one chunk of rows to the end of the table. */
  addSegment(columns: SegmentColumn[], length: number): void {
    this.segments.push({ start: this.rows_, length, columns })
    this.rows_ += length
  }

  /** The segment that holds one row, or nothing when the row is past the end. */
  private segmentOf(row: number): Segment | null {
    const held = this.segments[this.lastSegment]
    if (held && row >= held.start && row < held.start + held.length) {
      return held
    }
    for (let index = 0; index < this.segments.length; index += 1) {
      const segment = this.segments[index]!
      if (row >= segment.start && row < segment.start + segment.length) {
        this.lastSegment = index
        return segment
      }
    }
    return null
  }

  /** The value of one cell. A place that holds no value gives null. */
  cell(row: number, column: number): CellValue {
    const segment = this.segmentOf(row)
    if (!segment) {
      return null
    }
    if (segment.plain) {
      return segment.plain[row - segment.start]?.[column] ?? null
    }
    const values = segment.columns[column]
    if (!values) {
      return null
    }
    return valueOf(values, row - segment.start)
  }

  /**
   * One row as an array of values. A row of plain rows that holds more values
   * than the result names columns keeps every value it holds.
   */
  row(index: number): CellValue[] {
    const segment = this.segmentOf(index)
    const held = segment?.plain?.[index - segment.start]
    const width = Math.max(this.columns.length, held?.length ?? 0)
    const row: CellValue[] = new Array(width)
    for (let column = 0; column < width; column += 1) {
      row[column] = this.cell(index, column)
    }
    return row
  }

  /** The rows from one place to another, for the window of the grid. */
  slice(from: number, to: number): CellValue[][] {
    const rows: CellValue[][] = []
    for (let index = from; index < Math.min(to, this.rows_); index += 1) {
      rows.push(this.row(index))
    }
    return rows
  }

  /** Every row in turn. One row stands in memory at a time. */
  *rows(): Generator<CellValue[]> {
    for (let index = 0; index < this.rows_; index += 1) {
      yield this.row(index)
    }
  }
}

/** Reads one value of one column of one chunk. */
function valueOf(column: SegmentColumn, row: number): CellValue {
  switch (column.kind) {
    case 'null':
      return null
    case 'bool':
      return bitSet(column.nulls, row) ? null : bitSet(column.values, row)
    case 'int32':
      return bitSet(column.nulls, row) ? null : (column.values[row] ?? null)
    case 'float64':
      return bitSet(column.nulls, row) ? null : (column.values[row] ?? null)
    default:
      return textValue(column, row)
  }
}

/** Reads one value of a column of text or of JSON, and keeps it. */
function textValue(
  column: Extract<SegmentColumn, { kind: 'text' | 'json' }>,
  row: number,
): CellValue {
  const held = column.cache[row]
  if (held !== undefined) {
    return held
  }
  if (bitSet(column.nulls, row)) {
    column.cache[row] = null
    return null
  }
  const end = column.ends[row] ?? 0
  const start = row === 0 ? 0 : (column.ends[row - 1] ?? 0)
  const text = decoder.decode(column.bytes.subarray(start, end))
  const value: CellValue = column.kind === 'json' ? (JSON.parse(text) as CellValue) : text
  column.cache[row] = value
  return value
}

/** What the last frame of a run reports. */
export interface RunEnd {
  messages: Message[]
  rowsAffected: number | null
  elapsedMs: number
  stats: QueryStats | null
}

/** What the reader of a run tells its caller. */
export interface ResultStreamHandlers {
  /** A result set has ended, with every row it holds. */
  onSet: (table: ResultTable) => void
  /** The run has ended. */
  onEnd: (end: RunEnd) => void
}

/**
 * Reads the frames of one run. The caller hands over each message of the
 * channel as it arrives, and the reader calls the handlers.
 */
export class ResultStream {
  private readonly handlers: ResultStreamHandlers
  /** The set that each number of the backend names. */
  private readonly open = new Map<number, ResultTable>()

  constructor(handlers: ResultStreamHandlers) {
    this.handlers = handlers
  }

  /** Reads one message, which holds one or more frames. */
  feed(buffer: ArrayBuffer): void {
    const view = new DataView(buffer)
    let at = 0
    while (at < buffer.byteLength) {
      const kind = view.getUint8(at)
      at += 1
      switch (kind) {
        case FRAME_BEGIN_SET:
          at = this.readBeginSet(view, buffer, at)
          break
        case FRAME_CHUNK:
          at = this.readChunk(view, buffer, at)
          break
        case FRAME_END_SET:
          at = this.readEndSet(view, at)
          break
        case FRAME_END:
          at = this.readEnd(view, buffer, at)
          break
        default:
          throw new Error(`The rows hold a frame of the unknown kind ${kind}.`)
      }
    }
  }

  private readBeginSet(view: DataView, buffer: ArrayBuffer, at: number): number {
    const set = view.getUint32(at, true)
    const count = view.getUint32(at + 4, true)
    let cursor = at + 8
    const columns: ColumnInfo[] = []
    for (let index = 0; index < count; index += 1) {
      const name = readText(view, buffer, cursor)
      const typeName = readText(view, buffer, name.at)
      columns.push({ name: name.text, typeName: typeName.text })
      cursor = typeName.at
    }
    this.open.set(set, new ResultTable(columns))
    return cursor
  }

  private readChunk(view: DataView, buffer: ArrayBuffer, at: number): number {
    const set = view.getUint32(at, true)
    const rows = view.getUint32(at + 4, true)
    const count = view.getUint32(at + 8, true)
    let cursor = at + 12
    const columns: SegmentColumn[] = []
    for (let index = 0; index < count; index += 1) {
      const encoding = view.getUint8(cursor)
      cursor += 1
      const read = readColumn(view, buffer, cursor, encoding, rows)
      columns.push(read.column)
      cursor = read.at
    }
    this.open.get(set)?.addSegment(columns, rows)
    return cursor
  }

  private readEndSet(view: DataView, at: number): number {
    const set = view.getUint32(at, true)
    const truncated = view.getUint8(at + 4) === 1
    const table = this.open.get(set)
    if (table) {
      table.truncated = truncated
      this.open.delete(set)
      this.handlers.onSet(table)
    }
    return at + 5
  }

  private readEnd(view: DataView, buffer: ArrayBuffer, at: number): number {
    const json = readText(view, buffer, at)
    this.handlers.onEnd(JSON.parse(json.text) as RunEnd)
    return json.at
  }
}

/** Reads a length and the text that follows it. */
function readText(view: DataView, buffer: ArrayBuffer, at: number): { text: string; at: number } {
  const length = view.getUint32(at, true)
  const start = at + 4
  const text = decoder.decode(new Uint8Array(buffer, start, length))
  return { text, at: start + length }
}

/** The mask of the nulls of one column, one bit for each row. */
function readMask(buffer: ArrayBuffer, at: number, rows: number): { mask: Uint8Array; at: number } {
  const bytes = Math.ceil(rows / 8)
  return { mask: new Uint8Array(buffer, at, bytes), at: at + bytes }
}

/** Moves the place forward until the width divides it. */
function align(at: number, width: number): number {
  return at % width === 0 ? at : at + (width - (at % width))
}

/** Reads the values of one column of one chunk. */
function readColumn(
  view: DataView,
  buffer: ArrayBuffer,
  at: number,
  encoding: number,
  rows: number,
): { column: SegmentColumn; at: number } {
  if (encoding === ENCODING_NULL) {
    return { column: { kind: 'null' }, at }
  }
  if (encoding > ENCODING_JSON) {
    throw new Error(`The rows hold a column of the unknown form ${encoding}.`)
  }
  const nulls = readMask(buffer, at, rows)
  switch (encoding) {
    case ENCODING_BOOL: {
      const values = readMask(buffer, nulls.at, rows)
      return {
        column: { kind: 'bool', nulls: nulls.mask, values: values.mask },
        at: values.at,
      }
    }
    case ENCODING_INT32: {
      const start = align(nulls.at, 4)
      return {
        column: { kind: 'int32', nulls: nulls.mask, values: new Int32Array(buffer, start, rows) },
        at: start + rows * 4,
      }
    }
    case ENCODING_FLOAT64: {
      const start = align(nulls.at, 8)
      return {
        column: {
          kind: 'float64',
          nulls: nulls.mask,
          values: new Float64Array(buffer, start, rows),
        },
        at: start + rows * 8,
      }
    }
    case ENCODING_TEXT:
    case ENCODING_JSON: {
      const start = align(nulls.at, 4)
      const ends = new Uint32Array(buffer, start, rows)
      const lengthAt = start + rows * 4
      const length = view.getUint32(lengthAt, true)
      const bytes = new Uint8Array(buffer, lengthAt + 4, length)
      return {
        column: {
          kind: encoding === ENCODING_JSON ? 'json' : 'text',
          nulls: nulls.mask,
          ends,
          bytes,
          cache: new Array(rows),
        },
        at: lengthAt + 4 + length,
      }
    }
    default:
      throw new Error(`The rows hold a column of the unknown form ${encoding}.`)
  }
}
