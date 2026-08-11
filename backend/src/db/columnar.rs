//! The binary form that carries the rows of one execution to the user
//! interface.
//!
//! The rows travel in chunks, and each chunk holds its values column by
//! column. A column of numbers becomes a run of bytes that the frontend reads
//! as a typed array, and a column of text becomes one buffer of UTF-8 bytes
//! with an end offset for each value. The frontend therefore holds the bytes
//! of the answer once, and it builds no object for a row and no object for a
//! cell.
//!
//! Every number in the form is little-endian. One message holds one or more
//! frames, and each frame starts with one byte that names its kind.

use crate::db::sink::{RowSink, RunSummary, SinkControl};
use crate::db::{ColumnInfo, Message, QueryStats};
use crate::error::{Error, Result};
use serde::Serialize;
use serde_json::Value as JsonValue;
use tauri::ipc::{Channel, InvokeResponseBody};

/// The kind byte of each frame.
pub const FRAME_BEGIN_SET: u8 = 1;
pub const FRAME_CHUNK: u8 = 2;
pub const FRAME_END_SET: u8 = 3;
pub const FRAME_END: u8 = 4;

/// How the values of one column of one chunk are held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnEncoding {
    /// Every value of the chunk is null.
    Null = 0,
    /// One bit for each value, beside the mask of the nulls.
    Bool = 1,
    /// Four bytes for each value.
    Int32 = 2,
    /// Eight bytes for each value.
    Float64 = 3,
    /// One buffer of UTF-8 bytes, with an end offset for each value.
    Text = 4,
    /// As `Text`, and each value holds the JSON of the value.
    Json = 5,
}

/// Picks the encoding of one column from the values of one chunk. The values
/// arrive as JSON, so a column can hold anything, and the encoding therefore
/// belongs to the chunk and not to the column of the result.
pub fn pick_encoding<'a>(values: impl Iterator<Item = &'a JsonValue> + Clone) -> ColumnEncoding {
    let mut present = values.clone().filter(|value| !value.is_null()).peekable();
    if present.peek().is_none() {
        return ColumnEncoding::Null;
    }
    let present = values.filter(|value| !value.is_null());
    let mut all_bool = true;
    let mut all_int32 = true;
    let mut all_number = true;
    let mut all_text = true;
    for value in present {
        match value {
            JsonValue::Bool(_) => {
                all_int32 = false;
                all_number = false;
                all_text = false;
            }
            JsonValue::Number(number) => {
                all_bool = false;
                all_text = false;
                match number.as_i64() {
                    Some(whole) => {
                        if i32::try_from(whole).is_err() {
                            all_int32 = false;
                        }
                    }
                    None => all_int32 = false,
                }
                if number.as_f64().is_none() {
                    all_number = false;
                }
            }
            JsonValue::String(_) => {
                all_bool = false;
                all_int32 = false;
                all_number = false;
            }
            _ => {
                all_bool = false;
                all_int32 = false;
                all_number = false;
                all_text = false;
            }
        }
    }
    if all_bool {
        ColumnEncoding::Bool
    } else if all_int32 {
        ColumnEncoding::Int32
    } else if all_number {
        ColumnEncoding::Float64
    } else if all_text {
        ColumnEncoding::Text
    } else {
        ColumnEncoding::Json
    }
}

/// Writes the frame that opens one result set.
pub fn write_begin_set(buffer: &mut Vec<u8>, set: u32, columns: &[ColumnInfo]) {
    buffer.push(FRAME_BEGIN_SET);
    buffer.extend_from_slice(&set.to_le_bytes());
    buffer.extend_from_slice(&(columns.len() as u32).to_le_bytes());
    for column in columns {
        write_text(buffer, &column.name);
        write_text(buffer, &column.type_name);
    }
}

/// Writes the frame that closes one result set.
pub fn write_end_set(buffer: &mut Vec<u8>, set: u32, truncated: bool) {
    buffer.push(FRAME_END_SET);
    buffer.extend_from_slice(&set.to_le_bytes());
    buffer.push(u8::from(truncated));
}

/// Writes the frame that ends the run. The JSON holds the messages of the
/// server and the numbers of the run.
pub fn write_end(buffer: &mut Vec<u8>, summary_json: &str) {
    buffer.push(FRAME_END);
    write_text(buffer, summary_json);
}

/// Writes one chunk of rows, column by column. Every row of the chunk must
/// hold one value for each column.
pub fn write_chunk(buffer: &mut Vec<u8>, set: u32, columns: usize, rows: &[Vec<JsonValue>]) {
    buffer.push(FRAME_CHUNK);
    buffer.extend_from_slice(&set.to_le_bytes());
    buffer.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    buffer.extend_from_slice(&(columns as u32).to_le_bytes());
    for index in 0..columns {
        let encoding = pick_encoding(rows.iter().map(|row| cell(row, index)));
        buffer.push(encoding as u8);
        match encoding {
            ColumnEncoding::Null => {}
            ColumnEncoding::Bool => write_bool_column(buffer, rows, index),
            ColumnEncoding::Int32 => write_int32_column(buffer, rows, index),
            ColumnEncoding::Float64 => write_float64_column(buffer, rows, index),
            ColumnEncoding::Text => write_text_column(buffer, rows, index, false),
            ColumnEncoding::Json => write_text_column(buffer, rows, index, true),
        }
    }
}

/// The value of one cell. A row that is short gives null, so a driver that
/// sends fewer values than columns cannot stop the run.
fn cell(row: &[JsonValue], index: usize) -> &JsonValue {
    row.get(index).unwrap_or(&JsonValue::Null)
}

/// Writes a length and then the bytes of a text.
fn write_text(buffer: &mut Vec<u8>, text: &str) {
    buffer.extend_from_slice(&(text.len() as u32).to_le_bytes());
    buffer.extend_from_slice(text.as_bytes());
}

/// Writes one bit for each row, and a set bit marks a null.
fn write_null_mask(buffer: &mut Vec<u8>, rows: &[Vec<JsonValue>], index: usize) {
    write_bits(buffer, rows, |row| cell(row, index).is_null());
}

/// Writes one bit for each row from the answer of the test.
fn write_bits(buffer: &mut Vec<u8>, rows: &[Vec<JsonValue>], set: impl Fn(&[JsonValue]) -> bool) {
    let mut byte = 0u8;
    for (position, row) in rows.iter().enumerate() {
        if set(row) {
            byte |= 1 << (position % 8);
        }
        if position % 8 == 7 {
            buffer.push(byte);
            byte = 0;
        }
    }
    if rows.len() % 8 != 0 {
        buffer.push(byte);
    }
}

/// Adds bytes until the length of the buffer divides by the width. The
/// frontend builds a typed array over the buffer of the message, and such an
/// array needs a start that the width divides.
fn pad_to(buffer: &mut Vec<u8>, width: usize) {
    while buffer.len() % width != 0 {
        buffer.push(0);
    }
}

fn write_bool_column(buffer: &mut Vec<u8>, rows: &[Vec<JsonValue>], index: usize) {
    write_null_mask(buffer, rows, index);
    write_bits(buffer, rows, |row| {
        cell(row, index).as_bool().unwrap_or(false)
    });
}

fn write_int32_column(buffer: &mut Vec<u8>, rows: &[Vec<JsonValue>], index: usize) {
    write_null_mask(buffer, rows, index);
    pad_to(buffer, 4);
    for row in rows {
        let value = cell(row, index).as_i64().unwrap_or(0) as i32;
        buffer.extend_from_slice(&value.to_le_bytes());
    }
}

fn write_float64_column(buffer: &mut Vec<u8>, rows: &[Vec<JsonValue>], index: usize) {
    write_null_mask(buffer, rows, index);
    pad_to(buffer, 8);
    for row in rows {
        let value = cell(row, index).as_f64().unwrap_or(0.0);
        buffer.extend_from_slice(&value.to_le_bytes());
    }
}

/// Writes the text of each value into one buffer, with an end offset for each
/// value. A null holds the offset of the value before it, so it holds no
/// bytes of its own.
fn write_text_column(buffer: &mut Vec<u8>, rows: &[Vec<JsonValue>], index: usize, as_json: bool) {
    write_null_mask(buffer, rows, index);
    pad_to(buffer, 4);
    let mut offsets: Vec<u32> = Vec::with_capacity(rows.len());
    let mut bytes: Vec<u8> = Vec::new();
    for row in rows {
        let value = cell(row, index);
        if !value.is_null() {
            match (as_json, value) {
                (false, JsonValue::String(text)) => bytes.extend_from_slice(text.as_bytes()),
                _ => bytes.extend_from_slice(value.to_string().as_bytes()),
            }
        }
        offsets.push(bytes.len() as u32);
    }
    for offset in offsets {
        buffer.extend_from_slice(&offset.to_le_bytes());
    }
    buffer.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buffer.extend_from_slice(&bytes);
}

/// The number of rows of one chunk. A chunk of this size holds enough rows to
/// keep the cost of a message small beside the rows it carries, and few enough
/// that the memory of one chunk stays small.
pub const CHUNK_ROWS: usize = 1000;

/// What the run reports once every set has ended.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunEnd<'a> {
    messages: &'a [Message],
    rows_affected: Option<u64>,
    elapsed_ms: u64,
    stats: Option<QueryStats>,
}

/// A sink that sends the rows to the user interface as binary chunks. It
/// keeps one chunk of rows at a time, so a result of many rows never stands
/// in memory as a whole.
pub struct ChunkSink {
    channel: Channel<InvokeResponseBody>,
    max_rows: usize,
    /// The number of the set that is open, from zero.
    set: u32,
    /// True while a set is open.
    open: bool,
    columns: usize,
    rows_in_set: usize,
    batch: Vec<Vec<JsonValue>>,
    truncated: bool,
    messages: Vec<Message>,
}

impl ChunkSink {
    pub fn new(channel: Channel<InvokeResponseBody>, max_rows: usize) -> Self {
        Self {
            channel,
            max_rows,
            set: 0,
            open: false,
            columns: 0,
            rows_in_set: 0,
            batch: Vec::new(),
            truncated: false,
            messages: Vec::new(),
        }
    }

    /// Sends the chunk that stands open, when it holds a row.
    fn flush(&mut self) -> Result<()> {
        if self.batch.is_empty() {
            return Ok(());
        }
        let mut buffer = Vec::new();
        write_chunk(&mut buffer, self.set, self.columns, &self.batch);
        self.batch.clear();
        self.send(buffer)
    }

    fn send(&self, buffer: Vec<u8>) -> Result<()> {
        self.channel
            .send(InvokeResponseBody::Raw(buffer))
            .map_err(|error| {
                Error::Anyhow(anyhow::anyhow!(
                    "The rows could not reach the window: {error}"
                ))
            })
    }

    /// Sends the frame that ends the run, with the messages of the server and
    /// the numbers of the run.
    pub fn finish(mut self, summary: RunSummary) -> Result<()> {
        self.flush()?;
        let end = RunEnd {
            messages: &self.messages,
            rows_affected: summary.rows_affected,
            elapsed_ms: summary.elapsed_ms,
            stats: summary.stats,
        };
        let json = serde_json::to_string(&end)?;
        let mut buffer = Vec::new();
        write_end(&mut buffer, &json);
        self.send(buffer)
    }
}

impl RowSink for ChunkSink {
    fn begin_set(&mut self, columns: Vec<ColumnInfo>) -> Result<()> {
        if self.open {
            self.set += 1;
        }
        self.open = true;
        self.columns = columns.len();
        self.rows_in_set = 0;
        self.truncated = false;
        let mut buffer = Vec::new();
        write_begin_set(&mut buffer, self.set, &columns);
        self.send(buffer)
    }

    fn row(&mut self, row: Vec<JsonValue>) -> Result<SinkControl> {
        if self.rows_in_set >= self.max_rows {
            self.truncated = true;
            return Ok(SinkControl::Stop);
        }
        self.batch.push(row);
        self.rows_in_set += 1;
        if self.batch.len() >= CHUNK_ROWS {
            self.flush()?;
        }
        Ok(SinkControl::Continue)
    }

    fn end_set(&mut self, truncated: bool) -> Result<()> {
        self.flush()?;
        let cut = self.truncated || truncated;
        let mut buffer = Vec::new();
        write_end_set(&mut buffer, self.set, cut);
        self.send(buffer)
    }

    fn message(&mut self, message: Message) {
        self.messages.push(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Reads the frames of a message back into values, so that a test can
    /// compare what it wrote with what it meant.
    struct Reader<'a> {
        bytes: &'a [u8],
        at: usize,
    }

    /// One frame that the reader gives back.
    #[derive(Debug, PartialEq)]
    enum Frame {
        BeginSet {
            set: u32,
            columns: Vec<(String, String)>,
        },
        Chunk {
            set: u32,
            encodings: Vec<ColumnEncoding>,
            rows: Vec<Vec<JsonValue>>,
        },
        EndSet {
            set: u32,
            truncated: bool,
        },
        End {
            summary: String,
        },
    }

    impl<'a> Reader<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self { bytes, at: 0 }
        }

        fn u8(&mut self) -> u8 {
            let value = self.bytes[self.at];
            self.at += 1;
            value
        }

        fn u32(&mut self) -> u32 {
            let value = u32::from_le_bytes(self.bytes[self.at..self.at + 4].try_into().unwrap());
            self.at += 4;
            value
        }

        fn text(&mut self) -> String {
            let length = self.u32() as usize;
            let text = String::from_utf8(self.bytes[self.at..self.at + length].to_vec()).unwrap();
            self.at += length;
            text
        }

        fn align(&mut self, width: usize) {
            while self.at % width != 0 {
                self.at += 1;
            }
        }

        fn bits(&mut self, rows: usize) -> Vec<bool> {
            let bytes = rows.div_ceil(8);
            let mut flags = Vec::with_capacity(rows);
            for position in 0..rows {
                let byte = self.bytes[self.at + position / 8];
                flags.push(byte & (1 << (position % 8)) != 0);
            }
            self.at += bytes;
            flags
        }

        fn frames(&mut self) -> Vec<Frame> {
            let mut frames = Vec::new();
            while self.at < self.bytes.len() {
                frames.push(self.frame());
            }
            frames
        }

        fn frame(&mut self) -> Frame {
            match self.u8() {
                FRAME_BEGIN_SET => {
                    let set = self.u32();
                    let count = self.u32() as usize;
                    let columns = (0..count).map(|_| (self.text(), self.text())).collect();
                    Frame::BeginSet { set, columns }
                }
                FRAME_CHUNK => self.chunk(),
                FRAME_END_SET => Frame::EndSet {
                    set: self.u32(),
                    truncated: self.u8() == 1,
                },
                FRAME_END => Frame::End {
                    summary: self.text(),
                },
                other => panic!("the frame {other} is unknown"),
            }
        }

        fn chunk(&mut self) -> Frame {
            let set = self.u32();
            let row_count = self.u32() as usize;
            let column_count = self.u32() as usize;
            let mut encodings = Vec::with_capacity(column_count);
            let mut rows: Vec<Vec<JsonValue>> = vec![Vec::new(); row_count];
            for _ in 0..column_count {
                let encoding = match self.u8() {
                    0 => ColumnEncoding::Null,
                    1 => ColumnEncoding::Bool,
                    2 => ColumnEncoding::Int32,
                    3 => ColumnEncoding::Float64,
                    4 => ColumnEncoding::Text,
                    5 => ColumnEncoding::Json,
                    other => panic!("the encoding {other} is unknown"),
                };
                encodings.push(encoding);
                let values = self.column(encoding, row_count);
                for (row, value) in rows.iter_mut().zip(values) {
                    row.push(value);
                }
            }
            Frame::Chunk {
                set,
                encodings,
                rows,
            }
        }

        fn column(&mut self, encoding: ColumnEncoding, rows: usize) -> Vec<JsonValue> {
            if encoding == ColumnEncoding::Null {
                return vec![JsonValue::Null; rows];
            }
            let nulls = self.bits(rows);
            match encoding {
                ColumnEncoding::Bool => {
                    let flags = self.bits(rows);
                    nulls
                        .iter()
                        .zip(flags)
                        .map(|(null, flag)| match null {
                            true => JsonValue::Null,
                            false => JsonValue::Bool(flag),
                        })
                        .collect()
                }
                ColumnEncoding::Int32 => {
                    self.align(4);
                    nulls
                        .iter()
                        .map(|null| {
                            let value = i32::from_le_bytes(
                                self.bytes[self.at..self.at + 4].try_into().unwrap(),
                            );
                            self.at += 4;
                            match null {
                                true => JsonValue::Null,
                                false => json!(value),
                            }
                        })
                        .collect()
                }
                ColumnEncoding::Float64 => {
                    self.align(8);
                    nulls
                        .iter()
                        .map(|null| {
                            let value = f64::from_le_bytes(
                                self.bytes[self.at..self.at + 8].try_into().unwrap(),
                            );
                            self.at += 8;
                            match null {
                                true => JsonValue::Null,
                                false => json!(value),
                            }
                        })
                        .collect()
                }
                _ => self.text_column(&nulls, encoding == ColumnEncoding::Json),
            }
        }

        fn text_column(&mut self, nulls: &[bool], as_json: bool) -> Vec<JsonValue> {
            self.align(4);
            let ends: Vec<u32> = (0..nulls.len()).map(|_| self.u32()).collect();
            let length = self.u32() as usize;
            let bytes = &self.bytes[self.at..self.at + length];
            self.at += length;
            let mut values = Vec::with_capacity(nulls.len());
            let mut start = 0usize;
            for (null, end) in nulls.iter().zip(ends) {
                let end = end as usize;
                let text = std::str::from_utf8(&bytes[start..end]).unwrap();
                start = end;
                values.push(match (null, as_json) {
                    (true, _) => JsonValue::Null,
                    (false, true) => serde_json::from_str(text).unwrap(),
                    (false, false) => JsonValue::String(text.to_string()),
                });
            }
            values
        }
    }

    fn columns() -> Vec<ColumnInfo> {
        vec![
            ColumnInfo::new("id", "int"),
            ColumnInfo::new("name", "text"),
        ]
    }

    #[test]
    fn the_frames_of_a_run_come_back_as_they_were_written() {
        let mut buffer = Vec::new();
        write_begin_set(&mut buffer, 0, &columns());
        let rows = vec![
            vec![json!(1), json!("one")],
            vec![json!(2), JsonValue::Null],
        ];
        write_chunk(&mut buffer, 0, 2, &rows);
        write_end_set(&mut buffer, 0, true);
        write_end(&mut buffer, "{\"elapsedMs\":8}");

        let frames = Reader::new(&buffer).frames();
        assert_eq!(
            frames,
            vec![
                Frame::BeginSet {
                    set: 0,
                    columns: vec![
                        ("id".to_string(), "int".to_string()),
                        ("name".to_string(), "text".to_string()),
                    ],
                },
                Frame::Chunk {
                    set: 0,
                    encodings: vec![ColumnEncoding::Int32, ColumnEncoding::Text],
                    rows,
                },
                Frame::EndSet {
                    set: 0,
                    truncated: true,
                },
                Frame::End {
                    summary: "{\"elapsedMs\":8}".to_string(),
                },
            ]
        );
    }

    #[test]
    fn each_kind_of_value_takes_the_encoding_that_holds_it() {
        let rows = vec![
            vec![
                JsonValue::Null,
                json!(true),
                json!(7),
                json!(3_000_000_000i64),
                json!(1.5),
                json!("text"),
                json!({ "a": 1 }),
            ],
            vec![
                JsonValue::Null,
                JsonValue::Null,
                json!(-8),
                json!(2),
                json!(2),
                JsonValue::Null,
                json!([1, 2]),
            ],
        ];
        let mut buffer = Vec::new();
        write_chunk(&mut buffer, 3, 7, &rows);

        let frames = Reader::new(&buffer).frames();
        let Frame::Chunk {
            set,
            encodings,
            rows: read,
        } = &frames[0]
        else {
            panic!("the frame is not a chunk");
        };
        assert_eq!(*set, 3);
        assert_eq!(
            *encodings,
            vec![
                ColumnEncoding::Null,
                ColumnEncoding::Bool,
                ColumnEncoding::Int32,
                ColumnEncoding::Float64,
                ColumnEncoding::Float64,
                ColumnEncoding::Text,
                ColumnEncoding::Json,
            ]
        );
        // A column of doubles gives every value back as a double, so the
        // whole numbers of those two columns come back with a point.
        assert_eq!(
            *read,
            vec![
                vec![
                    JsonValue::Null,
                    json!(true),
                    json!(7),
                    json!(3_000_000_000.0),
                    json!(1.5),
                    json!("text"),
                    json!({ "a": 1 }),
                ],
                vec![
                    JsonValue::Null,
                    JsonValue::Null,
                    json!(-8),
                    json!(2.0),
                    json!(2.0),
                    JsonValue::Null,
                    json!([1, 2]),
                ],
            ]
        );
    }

    #[test]
    fn a_column_of_many_rows_keeps_every_null_apart() {
        // Nine rows cross the bound of one byte of the mask.
        let rows: Vec<Vec<JsonValue>> = (0..9)
            .map(|index| {
                vec![match index % 3 {
                    0 => JsonValue::Null,
                    _ => json!(index),
                }]
            })
            .collect();
        let mut buffer = Vec::new();
        write_chunk(&mut buffer, 0, 1, &rows);

        let frames = Reader::new(&buffer).frames();
        let Frame::Chunk { rows: read, .. } = &frames[0] else {
            panic!("the frame is not a chunk");
        };
        assert_eq!(*read, rows);
    }

    #[test]
    fn a_row_that_holds_too_few_values_gives_nulls() {
        let rows = vec![vec![json!(1)], vec![]];
        let mut buffer = Vec::new();
        write_chunk(&mut buffer, 0, 2, &rows);

        let frames = Reader::new(&buffer).frames();
        let Frame::Chunk { rows: read, .. } = &frames[0] else {
            panic!("the frame is not a chunk");
        };
        assert_eq!(
            *read,
            vec![
                vec![json!(1), JsonValue::Null],
                vec![JsonValue::Null, JsonValue::Null],
            ]
        );
    }

    #[test]
    fn a_number_that_no_double_holds_becomes_its_json() {
        // A number of more than eighteen digits does not fit a double, and
        // serde keeps it as text of its own.
        let rows = vec![vec![json!(u64::MAX)]];
        let mut buffer = Vec::new();
        write_chunk(&mut buffer, 0, 1, &rows);
        let frames = Reader::new(&buffer).frames();
        let Frame::Chunk { encodings, .. } = &frames[0] else {
            panic!("the frame is not a chunk");
        };
        assert_eq!(*encodings, vec![ColumnEncoding::Float64]);
    }

    #[test]
    fn a_chunk_without_rows_holds_the_null_encoding_of_each_column() {
        let mut buffer = Vec::new();
        write_chunk(&mut buffer, 1, 2, &[]);
        let frames = Reader::new(&buffer).frames();
        assert_eq!(
            frames,
            vec![Frame::Chunk {
                set: 1,
                encodings: vec![ColumnEncoding::Null, ColumnEncoding::Null],
                rows: Vec::new(),
            }]
        );
    }

    /// The messages that a channel of a test kept.
    type Kept = std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>;

    /// A channel that keeps every message, and the frames of those messages.
    fn collecting_channel() -> (Channel<InvokeResponseBody>, Kept) {
        let held: Kept = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = held.clone();
        let channel = Channel::new(move |body| {
            let bytes = match body {
                InvokeResponseBody::Raw(bytes) => bytes,
                InvokeResponseBody::Json(text) => text.into_bytes(),
            };
            kept.lock().unwrap().push(bytes);
            Ok(())
        });
        (channel, held)
    }

    /// The frames of every message of a channel, in the order they arrived.
    fn frames_of(messages: &[Vec<u8>]) -> Vec<Frame> {
        messages
            .iter()
            .flat_map(|message| Reader::new(message).frames())
            .collect()
    }

    #[test]
    fn the_sink_sends_a_set_as_frames() {
        let (channel, messages) = collecting_channel();
        let mut sink = ChunkSink::new(channel, 10);
        sink.begin_set(columns()).unwrap();
        assert_eq!(
            sink.row(vec![json!(1), json!("one")]).unwrap(),
            SinkControl::Continue
        );
        sink.message(Message::info("1 row returned."));
        sink.end_set(false).unwrap();
        sink.finish(RunSummary {
            rows_affected: Some(0),
            elapsed_ms: 8,
            stats: None,
        })
        .unwrap();

        let frames = frames_of(&messages.lock().unwrap());
        assert_eq!(frames.len(), 4);
        assert!(matches!(frames[0], Frame::BeginSet { set: 0, .. }));
        assert_eq!(
            frames[1],
            Frame::Chunk {
                set: 0,
                encodings: vec![ColumnEncoding::Int32, ColumnEncoding::Text],
                rows: vec![vec![json!(1), json!("one")]],
            }
        );
        assert_eq!(
            frames[2],
            Frame::EndSet {
                set: 0,
                truncated: false,
            }
        );
        let Frame::End { summary } = &frames[3] else {
            panic!("the last frame does not end the run");
        };
        let value: JsonValue = serde_json::from_str(summary).unwrap();
        assert_eq!(value["elapsedMs"], 8);
        assert_eq!(value["rowsAffected"], 0);
        assert_eq!(value["messages"][0]["text"], "1 row returned.");
    }

    #[test]
    fn the_sink_sends_one_chunk_for_each_thousand_rows() {
        let (channel, messages) = collecting_channel();
        let mut sink = ChunkSink::new(channel, 10_000);
        sink.begin_set(vec![ColumnInfo::new("n", "int")]).unwrap();
        for index in 0..(CHUNK_ROWS + 1) {
            sink.row(vec![json!(index as i64)]).unwrap();
        }
        sink.end_set(false).unwrap();

        let frames = frames_of(&messages.lock().unwrap());
        let chunks: Vec<&Frame> = frames
            .iter()
            .filter(|frame| matches!(frame, Frame::Chunk { .. }))
            .collect();
        assert_eq!(chunks.len(), 2);
        let Frame::Chunk { rows: first, .. } = chunks[0] else {
            panic!("the frame is not a chunk");
        };
        let Frame::Chunk { rows: second, .. } = chunks[1] else {
            panic!("the frame is not a chunk");
        };
        assert_eq!(first.len(), CHUNK_ROWS);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0], vec![json!(CHUNK_ROWS as i64)]);
    }

    #[test]
    fn the_sink_stops_the_read_at_the_row_limit_and_marks_the_set() {
        let (channel, messages) = collecting_channel();
        let mut sink = ChunkSink::new(channel, 2);
        sink.begin_set(vec![ColumnInfo::new("n", "int")]).unwrap();
        assert_eq!(sink.row(vec![json!(1)]).unwrap(), SinkControl::Continue);
        assert_eq!(sink.row(vec![json!(2)]).unwrap(), SinkControl::Continue);
        assert_eq!(sink.row(vec![json!(3)]).unwrap(), SinkControl::Stop);
        sink.end_set(false).unwrap();

        let frames = frames_of(&messages.lock().unwrap());
        let Frame::Chunk { rows, .. } = &frames[1] else {
            panic!("the frame is not a chunk");
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(
            frames[2],
            Frame::EndSet {
                set: 0,
                truncated: true,
            }
        );
    }

    #[test]
    fn the_sink_counts_the_sets_of_a_script() {
        let (channel, messages) = collecting_channel();
        let mut sink = ChunkSink::new(channel, 10);
        sink.begin_set(vec![ColumnInfo::new("a", "int")]).unwrap();
        sink.row(vec![json!(1)]).unwrap();
        sink.end_set(false).unwrap();
        sink.begin_set(vec![ColumnInfo::new("b", "int")]).unwrap();
        sink.row(vec![json!(2)]).unwrap();
        sink.end_set(true).unwrap();
        sink.finish(RunSummary::default()).unwrap();

        let frames = frames_of(&messages.lock().unwrap());
        let sets: Vec<u32> = frames
            .iter()
            .filter_map(|frame| match frame {
                Frame::EndSet { set, .. } => Some(*set),
                _ => None,
            })
            .collect();
        assert_eq!(sets, vec![0, 1]);
        // The limit of the first set does not reach the second one.
        assert_eq!(
            frames
                .iter()
                .filter(|frame| matches!(
                    frame,
                    Frame::EndSet {
                        truncated: true,
                        ..
                    }
                ))
                .count(),
            1
        );
    }

    #[test]
    fn a_set_without_a_row_sends_no_chunk() {
        let (channel, messages) = collecting_channel();
        let mut sink = ChunkSink::new(channel, 10);
        sink.begin_set(columns()).unwrap();
        sink.end_set(false).unwrap();
        sink.finish(RunSummary::default()).unwrap();

        let frames = frames_of(&messages.lock().unwrap());
        assert!(!frames
            .iter()
            .any(|frame| matches!(frame, Frame::Chunk { .. })));
    }

    #[test]
    fn a_channel_that_refuses_a_message_gives_an_error() {
        let channel = Channel::new(|_body| Err(tauri::Error::WebviewNotFound));
        let mut sink = ChunkSink::new(channel, 10);
        let error = sink.begin_set(columns()).err().unwrap();
        assert_eq!(error.kind(), crate::error::ErrorKind::Internal);
    }

    #[test]
    fn the_bytes_of_a_small_chunk_stand_as_the_frontend_expects_them() {
        // The frontend holds the same case, so the two sides keep one form.
        let mut buffer = Vec::new();
        write_chunk(&mut buffer, 0, 1, &[vec![json!(1)], vec![json!(-2)]]);
        assert_eq!(
            buffer,
            vec![
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
                ColumnEncoding::Int32 as u8,
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
            ]
        );
    }
}
