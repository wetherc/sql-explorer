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

import type { CellValue, ColumnInfo, KeptSet, Message, QueryStats } from '@/types/api'
import { exactAsNumber } from './format'

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
const ENCODING_DICT = 6

const decoder = new TextDecoder()

/**
 * The most that the kept values of the text columns of one table weigh, as
 * the weight that `cacheWeight` gives. A grid that scrolls through a large
 * result reads each chunk once, and without a limit the texts of every chunk
 * it read stay in memory beside the bytes of that chunk.
 */
export const CACHE_WEIGHT = 16 * 1024 * 1024
/** The weight of one kept value besides its bytes: its slot and its object. */
const CACHE_CELL_WEIGHT = 16

/** One column of one chunk, in the form the bytes carry. */
type SegmentColumn =
  | { encoding: 'null' }
  | { encoding: 'bool'; nulls: Uint8Array; values: Uint8Array }
  | { encoding: 'int32'; nulls: Uint8Array; values: Int32Array }
  | { encoding: 'float64'; nulls: Uint8Array; values: Float64Array }
  | {
      encoding: 'text' | 'json'
      nulls: Uint8Array
      ends: Uint32Array
      bytes: Uint8Array
      /**
       * The values that were read already, or null when the table keeps
       * none of them. The table drops the cache of a column that no read
       * used for a while, when the caches pass their limit.
       */
      cache: Array<CellValue | undefined> | null
      /** True when a read used the cache after the last pass of the limit. */
      used: boolean
    }
  | {
      encoding: 'dict'
      nulls: Uint8Array
      /** The place in the dictionary of the text of each row. */
      codes: Uint32Array
      /** The end of each text of the dictionary in `bytes`. */
      ends: Uint32Array
      bytes: Uint8Array
      /** The texts that were read already, one for each different text. */
      cache: Array<string | undefined>
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

type TextColumn = Extract<SegmentColumn, { encoding: 'text' | 'json' }>

function isText(column: SegmentColumn): column is TextColumn {
  return column.encoding === 'text' || column.encoding === 'json'
}

/** The weight that the cache of one text column adds to the table. */
function cacheWeight(column: TextColumn): number {
  return column.bytes.byteLength + column.ends.length * CACHE_CELL_WEIGHT
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
  /** The text columns that keep a cache, the oldest first. */
  private readonly cached = new Set<TextColumn>()
  /** The weight of the caches of `cached`. */
  private cachedWeight = 0
  private readonly cacheLimit: number

  /** `cacheLimit` is the most that the caches of the text columns weigh. */
  constructor(columns: ColumnInfo[], cacheLimit = CACHE_WEIGHT) {
    this.columns = columns
    this.cacheLimit = cacheLimit
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

  /**
   * The segment that holds one row, or nothing when the row is past the end.
   * The segments stand in the order of their first rows, so a search that
   * halves the list finds the segment of a table of many chunks in few steps.
   */
  private segmentOf(row: number): Segment | null {
    const held = this.segments[this.lastSegment]
    if (held && row >= held.start && row < held.start + held.length) {
      return held
    }
    if (row < 0 || row >= this.rows_) {
      return null
    }
    let low = 0
    let high = this.segments.length - 1
    // The row is inside the table, so some segment holds it, and the search
    // ends with `low` at the last segment that starts at or before the row.
    while (low < high) {
      const middle = (low + high + 1) >> 1
      if (this.segments[middle]!.start <= row) {
        low = middle
      } else {
        high = middle - 1
      }
    }
    this.lastSegment = low
    return this.segments[low]!
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
    if (isText(values)) {
      return textValue(values, this.cacheOf(values), row - segment.start)
    }
    return valueOf(values, row - segment.start)
  }

  /**
   * The cache of one text column. A column without one gets a new cache, and
   * the table then drops old caches until their weight is under the limit.
   * The drop gives each cache that a read used a second turn, so the chunks
   * on screen keep their texts while a scroll passes over others.
   */
  private cacheOf(column: TextColumn): Array<CellValue | undefined> {
    column.used = true
    if (column.cache) {
      return column.cache
    }
    const cache: Array<CellValue | undefined> = new Array(column.ends.length)
    column.cache = cache
    this.cached.add(column)
    this.cachedWeight += cacheWeight(column)
    // A set visits the entries that join it while the loop runs, so a column
    // that goes to the end for its second turn comes up again. Each column
    // comes up at most twice, because its mark is clear on its second turn.
    for (const held of this.cached) {
      if (this.cachedWeight <= this.cacheLimit || this.cached.size === 1) {
        break
      }
      this.cached.delete(held)
      if (held.used) {
        held.used = false
        this.cached.add(held)
      } else {
        held.cache = null
        this.cachedWeight -= cacheWeight(held)
      }
    }
    return cache
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

/** Reads one value of one column of one chunk that holds no text of its own. */
function valueOf(column: Exclude<SegmentColumn, TextColumn>, row: number): CellValue {
  switch (column.encoding) {
    case 'null':
      return null
    case 'bool':
      return bitSet(column.nulls, row) ? null : bitSet(column.values, row)
    case 'int32':
      return bitSet(column.nulls, row) ? null : (column.values[row] ?? null)
    case 'float64':
      return bitSet(column.nulls, row) ? null : (column.values[row] ?? null)
    default:
      return dictValue(column, row)
  }
}

/**
 * Reads one value of a column that holds each of its texts once. Every row of
 * one text gives back that one text, so a column of many rows costs one text
 * for each different value.
 */
function dictValue(column: Extract<SegmentColumn, { encoding: 'dict' }>, row: number): CellValue {
  if (bitSet(column.nulls, row)) {
    return null
  }
  const code = column.codes[row] ?? 0
  if (code >= column.ends.length) {
    throw new Error(
      `The result refers to dictionary entry ${code}, but the dictionary has only ${column.ends.length} entries.`,
    )
  }
  const held = column.cache[code]
  if (held !== undefined) {
    return held
  }
  const end = column.ends[code] ?? 0
  const start = code === 0 ? 0 : (column.ends[code - 1] ?? 0)
  const text = decoder.decode(column.bytes.subarray(start, end))
  column.cache[code] = text
  return text
}

/**
 * True when a JavaScript number keeps every digit of a JSON number and the
 * grid shows it as the server wrote it. A number with no exponent must come
 * back as the same text, so `1.0`, `10.50` and `-0` keep their zeros.
 */
function keepsForm(token: string): boolean {
  return exactAsNumber(token) && (/[eE]/.test(token) || String(Number(token)) === token)
}

/**
 * Reads one JSON value. A value that contains a number that a JavaScript
 * number cannot keep with every digit, or whose zeros a number drops, stays
 * the text that the server sent, so an array of bigint values or a jsonb
 * document shows its digits.
 */
export function parseJsonCell(text: string): CellValue {
  // A string in quotes is skipped, and each number outside one is checked.
  for (const [token] of text.matchAll(/"(?:[^"\\]|\\.)*"|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g)) {
    if (!token.startsWith('"') && !keepsForm(token)) {
      return text
    }
  }
  return JSON.parse(text) as CellValue
}

/** Reads one value of a column of text or of JSON, and keeps it in `cache`. */
function textValue(
  column: TextColumn,
  cache: Array<CellValue | undefined>,
  row: number,
): CellValue {
  const held = cache[row]
  if (held !== undefined) {
    return held
  }
  if (bitSet(column.nulls, row)) {
    cache[row] = null
    return null
  }
  const end = column.ends[row] ?? 0
  const start = row === 0 ? 0 : (column.ends[row - 1] ?? 0)
  const text = decoder.decode(column.bytes.subarray(start, end))
  const value: CellValue = column.encoding === 'json' ? parseJsonCell(text) : text
  cache[row] = value
  return value
}

/** What the last frame of a run reports. */
export interface RunEnd {
  messages: Message[]
  rowsAffected: number | null
  elapsedMs: number
  stats: QueryStats | null
  /** The sets that the row limit cut and whose full rows the backend kept.
   *  A plan gives none. */
  kept?: KeptSet[]
  /** True when the tab's session closed before or during the run and a new
   *  session took its place. */
  sessionReset?: boolean
  /** True when the tab's session is inside an open transaction after the
   *  run. Missing when the backend doesn't know. */
  openTransaction?: boolean
}

/** What the reader of a run tells its caller. */
export interface ResultStreamHandlers {
  /** A result set has opened. The table holds no row yet and fills while
   *  the set streams, so the interface can show the rows as they arrive. */
  onBegin?: (table: ResultTable) => void
  /** A chunk of rows joined the table of an open set. */
  onRows?: (table: ResultTable) => void
  /** A result set has ended, with every row it holds. */
  onSet: (table: ResultTable) => void
  /** The server sent a message while the run goes on. */
  onMessage?: (message: Message) => void
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
  /** The first fault of the frames, when one came. */
  private fault: Error | null = null
  /** True once the frame that ends the run has arrived. */
  private ended = false
  /** True once the caller takes no more messages for this run. */
  private closed = false
  /** The caller that waits for the end of the run, when one waits. */
  private waiter: (() => void) | null = null
  /** The timer that ends a wait in which no message arrives. */
  private idleTimer: ReturnType<typeof setTimeout> | null = null
  private idleMs = 0

  constructor(handlers: ResultStreamHandlers) {
    this.handlers = handlers
  }

  /**
   * The fault of the frames, when one came. The caller of the run reads it
   * once the backend answers and reports it to the user.
   */
  get failure(): Error | null {
    return this.fault
  }

  /**
   * Reads one message, which holds one or more frames.
   *
   * A fault of the frames is kept and not thrown, because the caller is the
   * channel of the bridge and a throw there reaches nobody. The reader also
   * takes no later message, because a message that it cannot read leaves the
   * place in the frames unknown.
   */
  feed(buffer: ArrayBuffer): void {
    if (this.fault !== null || this.closed) {
      return
    }
    try {
      this.readFrames(buffer)
    } catch (error) {
      this.fault = error instanceof Error ? error : new Error(String(error))
    }
    this.wake()
  }

  /**
   * Waits for the frame that ends the run, or for a fault of the frames.
   *
   * The bridge sends a message of 1024 bytes or more through a fetch of its
   * own, so the last frames can arrive after the command answers. The bridge
   * keeps the order of the messages, so a message that it loses stops every
   * later one. A wait in which no message arrives for `idleMs` therefore ends
   * with a fault, and the run does not wait without end.
   */
  settle(idleMs: number): Promise<void> {
    if (this.settled) {
      return Promise.resolve()
    }
    this.idleMs = idleMs
    return new Promise((resolve) => {
      this.waiter = resolve
      this.armIdle()
    })
  }

  /**
   * Takes no more messages. A frame that arrives later belongs to a run that
   * has ended, so the reader drops it and it does not reach the panes of the
   * next run.
   */
  close(): void {
    this.closed = true
    this.stopIdle()
    this.waiter = null
  }

  private get settled(): boolean {
    return this.ended || this.fault !== null
  }

  /** Ends the wait when the run has ended, or starts the idle time again. */
  private wake(): void {
    if (this.waiter === null) {
      return
    }
    if (this.settled) {
      const waiter = this.waiter
      this.close()
      waiter()
    } else {
      this.armIdle()
    }
  }

  private armIdle(): void {
    this.stopIdle()
    this.idleTimer = setTimeout(() => {
      this.fault = new Error('The last rows of the result never arrived.')
      this.wake()
    }, this.idleMs)
  }

  private stopIdle(): void {
    if (this.idleTimer !== null) {
      clearTimeout(this.idleTimer)
      this.idleTimer = null
    }
  }

  /** Walks the frames of one message. */
  private readFrames(buffer: ArrayBuffer): void {
    const view = new DataView(buffer)
    let at = 0
    while (at < buffer.byteLength) {
      const frameType = view.getUint8(at)
      at += 1
      switch (frameType) {
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
        case FRAME_MESSAGE:
          at = this.readMessage(view, buffer, at)
          break
        default:
          throw new Error(`The result contains a frame of unknown type ${frameType}.`)
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
    const table = new ResultTable(columns)
    this.open.set(set, table)
    this.handlers.onBegin?.(table)
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
    const table = this.open.get(set)
    if (table) {
      table.addSegment(columns, rows)
      this.handlers.onRows?.(table)
    }
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

  private readMessage(view: DataView, buffer: ArrayBuffer, at: number): number {
    const json = readText(view, buffer, at)
    this.handlers.onMessage?.(JSON.parse(json.text) as Message)
    return json.at
  }

  private readEnd(view: DataView, buffer: ArrayBuffer, at: number): number {
    const json = readText(view, buffer, at)
    this.ended = true
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
    return { column: { encoding: 'null' }, at }
  }
  // A form the reader does not know says nothing about the bytes that
  // follow it, so the walk of the frames stops here and not further along.
  if (encoding > ENCODING_DICT) {
    throw new Error(`The result contains a column with unknown encoding ${encoding}.`)
  }
  const nulls = readMask(buffer, at, rows)
  switch (encoding) {
    case ENCODING_BOOL: {
      const values = readMask(buffer, nulls.at, rows)
      return {
        column: { encoding: 'bool', nulls: nulls.mask, values: values.mask },
        at: values.at,
      }
    }
    case ENCODING_INT32: {
      const start = align(nulls.at, 4)
      return {
        column: {
          encoding: 'int32',
          nulls: nulls.mask,
          values: new Int32Array(buffer, start, rows),
        },
        at: start + rows * 4,
      }
    }
    case ENCODING_FLOAT64: {
      const start = align(nulls.at, 8)
      return {
        column: {
          encoding: 'float64',
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
          encoding: encoding === ENCODING_JSON ? 'json' : 'text',
          nulls: nulls.mask,
          ends,
          bytes,
          cache: null,
          used: false,
        },
        at: lengthAt + 4 + length,
      }
    }
    // The dictionary is the last form the reader knows, and a form it does
    // not know never reaches this far.
    default: {
      const countAt = align(nulls.at, 4)
      const count = view.getUint32(countAt, true)
      const ends = new Uint32Array(buffer, countAt + 4, count)
      const lengthAt = countAt + 4 + count * 4
      const length = view.getUint32(lengthAt, true)
      const bytes = new Uint8Array(buffer, lengthAt + 4, length)
      const codesAt = align(lengthAt + 4 + length, 4)
      return {
        column: {
          encoding: 'dict',
          nulls: nulls.mask,
          codes: new Uint32Array(buffer, codesAt, rows),
          ends,
          bytes,
          cache: new Array(count),
        },
        at: codesAt + rows * 4,
      }
    }
  }
}
