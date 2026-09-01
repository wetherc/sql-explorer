//! The reader of a `sql_variant` value.
//!
//! A `sql_variant` cell carries its own type. The bytes hold a total
//! length, the token of the base type, a count of property bytes, the
//! property bytes, and then the value. The value has no length of its own,
//! so its length comes from the total length less the two bytes of the
//! header and the property bytes.
//!
//! The reader gives back the `ColumnData` of the base type. A value of the
//! length zero is a null value.

use std::borrow::Cow;

use byteorder::{ByteOrder, LittleEndian};
use uuid::Uuid;

use crate::{
    error::Error,
    sql_read_bytes::SqlReadBytes,
    tds::{codec::guid, Collation, Numeric},
    ColumnData, FixedLenType, VarLenType,
};

/// The count of bytes that the header of a value holds: the token of the
/// base type and the count of the property bytes.
const HEADER_LEN: usize = 2;

pub(crate) async fn decode<R>(src: &mut R) -> crate::Result<ColumnData<'static>>
where
    R: SqlReadBytes + Unpin,
{
    let total = src.read_u32_le().await? as usize;

    if total == 0 {
        return Ok(ColumnData::String(None));
    }

    if total < HEADER_LEN {
        return Err(invalid(format!("a length of {total} is too short")));
    }

    let base = src.read_u8().await?;
    let props = src.read_u8().await? as usize;

    if total < HEADER_LEN + props {
        return Err(invalid(format!(
            "a length of {total} does not hold {props} property bytes"
        )));
    }

    let len = total - HEADER_LEN - props;

    if let Ok(ty) = FixedLenType::try_from(base) {
        expect_props(0, props)?;
        return fixed_len(src, ty, len).await;
    }

    match VarLenType::try_from(base) {
        Ok(ty) => var_len(src, ty, props, len).await,
        Err(()) => Err(invalid(format!("the base type {base:?} is not known"))),
    }
}

/// Reads a value of a base type that has one length alone.
async fn fixed_len<R>(
    src: &mut R,
    ty: FixedLenType,
    len: usize,
) -> crate::Result<ColumnData<'static>>
where
    R: SqlReadBytes + Unpin,
{
    let expected = match ty {
        FixedLenType::Bit | FixedLenType::Int1 => 1,
        FixedLenType::Int2 => 2,
        FixedLenType::Int4 | FixedLenType::Float4 | FixedLenType::Money4 => 4,
        FixedLenType::Datetime4 => 4,
        FixedLenType::Int8 | FixedLenType::Float8 | FixedLenType::Money => 8,
        FixedLenType::Datetime => 8,
        FixedLenType::Null => return Err(invalid("the base type null has no value".to_string())),
    };

    if len != expected {
        return Err(invalid(format!(
            "the base type {ty:?} asks for {expected} bytes and the value holds {len}"
        )));
    }

    let value = match ty {
        FixedLenType::Bit => ColumnData::Bit(Some(src.read_u8().await? > 0)),
        FixedLenType::Int1 => ColumnData::U8(Some(src.read_u8().await?)),
        FixedLenType::Int2 => ColumnData::I16(Some(src.read_i16_le().await?)),
        FixedLenType::Int4 => ColumnData::I32(Some(src.read_i32_le().await?)),
        FixedLenType::Int8 => ColumnData::I64(Some(src.read_i64_le().await?)),
        FixedLenType::Float4 => ColumnData::F32(Some(src.read_f32_le().await?)),
        FixedLenType::Float8 => ColumnData::F64(Some(src.read_f64_le().await?)),
        FixedLenType::Money => super::money::decode(src, 8).await?,
        FixedLenType::Money4 => super::money::decode(src, 4).await?,
        FixedLenType::Datetime => {
            ColumnData::DateTime(Some(crate::tds::time::DateTime::decode(src).await?))
        }
        FixedLenType::Datetime4 => {
            ColumnData::SmallDateTime(Some(crate::tds::time::SmallDateTime::decode(src).await?))
        }
        FixedLenType::Null => unreachable!("the null type stopped the read above"),
    };

    Ok(value)
}

/// Reads a value of a base type that carries property bytes, a length of
/// its own, or both.
async fn var_len<R>(
    src: &mut R,
    ty: VarLenType,
    props: usize,
    len: usize,
) -> crate::Result<ColumnData<'static>>
where
    R: SqlReadBytes + Unpin,
{
    match ty {
        VarLenType::Guid => {
            expect_props(0, props)?;
            let mut bytes = [0u8; 16];
            read_into(src, &mut bytes).await?;
            guid::reorder_bytes(&mut bytes);
            Ok(ColumnData::Guid(Some(Uuid::from_bytes(bytes))))
        }
        VarLenType::Decimaln | VarLenType::Numericn => {
            expect_props(2, props)?;
            let _precision = src.read_u8().await?;
            let scale = src.read_u8().await?;
            numeric(src, scale, len).await
        }
        #[cfg(feature = "tds73")]
        VarLenType::Daten => {
            expect_props(0, props)?;
            Ok(ColumnData::Date(Some(
                crate::tds::time::Date::decode(src).await?,
            )))
        }
        #[cfg(feature = "tds73")]
        VarLenType::Timen => {
            expect_props(1, props)?;
            let scale = src.read_u8().await? as usize;
            Ok(ColumnData::Time(Some(
                crate::tds::time::Time::decode(src, scale, len).await?,
            )))
        }
        #[cfg(feature = "tds73")]
        VarLenType::Datetime2 => {
            expect_props(1, props)?;
            let scale = src.read_u8().await? as usize;
            let time_len = len
                .checked_sub(3)
                .ok_or_else(|| invalid(format!("a datetime2 of {len} bytes is too short")))?;
            Ok(ColumnData::DateTime2(Some(
                crate::tds::time::DateTime2::decode(src, scale, time_len).await?,
            )))
        }
        #[cfg(feature = "tds73")]
        VarLenType::DatetimeOffsetn => {
            expect_props(1, props)?;
            let scale = src.read_u8().await? as usize;
            let time_len = len
                .checked_sub(5)
                .ok_or_else(|| invalid(format!("a datetimeoffset of {len} bytes is too short")))?;
            Ok(ColumnData::DateTimeOffset(Some(
                crate::tds::time::DateTimeOffset::decode(src, scale, time_len as u8).await?,
            )))
        }
        VarLenType::BigBinary | VarLenType::BigVarBin => {
            expect_props(2, props)?;
            src.read_u16_le().await?;
            let bytes = read_bytes(src, len).await?;
            Ok(ColumnData::Binary(Some(Cow::from(bytes))))
        }
        VarLenType::BigChar | VarLenType::BigVarChar => {
            expect_props(7, props)?;
            let collation = read_collation(src).await?;
            let bytes = read_bytes(src, len).await?;
            let encoder = collation.encoding()?;
            let text = encoder
                .decode_without_bom_handling_and_without_replacement(&bytes)
                .ok_or_else(|| Error::Encoding("invalid sequence".into()))?
                .to_string();
            Ok(ColumnData::String(Some(text.into())))
        }
        VarLenType::NChar | VarLenType::NVarchar => {
            expect_props(7, props)?;
            read_collation(src).await?;
            let bytes = read_bytes(src, len).await?;
            if bytes.len() % 2 != 0 {
                return Err(invalid(format!(
                    "a text of {} bytes holds a half unit",
                    bytes.len()
                )));
            }
            let units: Vec<u16> = bytes.chunks(2).map(LittleEndian::read_u16).collect();
            Ok(ColumnData::String(Some(String::from_utf16(&units)?.into())))
        }
        ty => Err(invalid(format!("the base type {ty:?} is not supported"))),
    }
}

/// Reads the digits of a decimal value. The value holds a byte of the sign
/// and then the digits, without a sign, in the little-endian order.
async fn numeric<R>(src: &mut R, scale: u8, len: usize) -> crate::Result<ColumnData<'static>>
where
    R: SqlReadBytes + Unpin,
{
    let digits = len
        .checked_sub(1)
        .ok_or_else(|| invalid("a decimal holds no digits".to_string()))?;

    if digits > 16 {
        return Err(invalid(format!("a decimal of {digits} bytes is too wide")));
    }

    let sign = match src.read_u8().await? {
        0 => -1i128,
        1 => 1i128,
        other => return Err(invalid(format!("a sign of {other} is not valid"))),
    };

    let bytes = read_bytes(src, digits).await?;
    let mut raw = [0u8; 16];
    raw[..bytes.len()].copy_from_slice(&bytes);

    let magnitude = i128::try_from(u128::from_le_bytes(raw))
        .map_err(|_| invalid("a decimal is wider than the target type".to_string()))?;

    Ok(ColumnData::Numeric(Some(Numeric::new_with_scale(
        magnitude * sign,
        scale,
    ))))
}

/// Reads the five bytes of a collation and skips the two bytes of the
/// greatest length, which the value does not need.
async fn read_collation<R>(src: &mut R) -> crate::Result<Collation>
where
    R: SqlReadBytes + Unpin,
{
    let info = src.read_u32_le().await?;
    let sort_id = src.read_u8().await?;
    src.read_u16_le().await?;
    Ok(Collation::new(info, sort_id))
}

/// Reads a count of bytes into a new buffer.
async fn read_bytes<R>(src: &mut R, len: usize) -> crate::Result<Vec<u8>>
where
    R: SqlReadBytes + Unpin,
{
    let mut bytes = vec![0u8; len];
    read_into(src, &mut bytes).await?;
    Ok(bytes)
}

/// Fills a buffer with the next bytes of the stream.
async fn read_into<R>(src: &mut R, bytes: &mut [u8]) -> crate::Result<()>
where
    R: SqlReadBytes + Unpin,
{
    for item in bytes.iter_mut() {
        *item = src.read_u8().await?;
    }
    Ok(())
}

/// Stops a read whose count of property bytes does not match the base type.
fn expect_props(expected: usize, got: usize) -> crate::Result<()> {
    if expected == got {
        return Ok(());
    }
    Err(invalid(format!(
        "the base type asks for {expected} property bytes and the value holds {got}"
    )))
}

/// Builds the error of a value that the reader cannot take.
fn invalid(reason: String) -> Error {
    Error::Protocol(format!("sql_variant: {reason}").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_read_bytes::test_utils::IntoSqlReadBytes;
    use bytes::{BufMut, BytesMut};

    /// Builds the bytes of one value: the total length, the token of the
    /// base type, the count of the property bytes, the property bytes, and
    /// the value.
    fn cell(base: u8, props: &[u8], value: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u32_le((HEADER_LEN + props.len() + value.len()) as u32);
        buf.put_u8(base);
        buf.put_u8(props.len() as u8);
        buf.put_slice(props);
        buf.put_slice(value);
        buf
    }

    /// Reads the bytes of one value.
    async fn read(buf: BytesMut) -> crate::Result<ColumnData<'static>> {
        decode(&mut buf.into_sql_read_bytes()).await
    }

    /// Gives the text of an error, so that a test can look into it.
    async fn error(buf: BytesMut) -> String {
        read(buf).await.unwrap_err().to_string()
    }

    /// The property bytes of a text value: the collation and the greatest
    /// length of the column.
    fn text_props(info: u32, sort_id: u8) -> Vec<u8> {
        let mut props = info.to_le_bytes().to_vec();
        props.push(sort_id);
        props.extend_from_slice(&8000u16.to_le_bytes());
        props
    }

    #[tokio::test]
    async fn a_value_of_no_length_is_null() {
        let mut buf = BytesMut::new();
        buf.put_u32_le(0);
        assert_eq!(read(buf).await.unwrap(), ColumnData::String(None));
    }

    #[tokio::test]
    async fn every_whole_number_keeps_its_width() {
        assert_eq!(
            read(cell(0x30, &[], &[7])).await.unwrap(),
            ColumnData::U8(Some(7))
        );
        assert_eq!(
            read(cell(0x34, &[], &(-2i16).to_le_bytes())).await.unwrap(),
            ColumnData::I16(Some(-2))
        );
        assert_eq!(
            read(cell(0x38, &[], &42i32.to_le_bytes())).await.unwrap(),
            ColumnData::I32(Some(42))
        );
        assert_eq!(
            read(cell(0x7F, &[], &9i64.to_le_bytes())).await.unwrap(),
            ColumnData::I64(Some(9))
        );
        assert_eq!(
            read(cell(0x32, &[], &[1])).await.unwrap(),
            ColumnData::Bit(Some(true))
        );
    }

    #[tokio::test]
    async fn a_float_and_a_money_keep_their_value() {
        assert_eq!(
            read(cell(0x3B, &[], &1.5f32.to_le_bytes())).await.unwrap(),
            ColumnData::F32(Some(1.5))
        );
        assert_eq!(
            read(cell(0x3E, &[], &2.5f64.to_le_bytes())).await.unwrap(),
            ColumnData::F64(Some(2.5))
        );

        let mut money = 0i32.to_le_bytes().to_vec();
        money.extend_from_slice(&10000u32.to_le_bytes());
        assert_eq!(
            read(cell(0x3C, &[], &money)).await.unwrap(),
            ColumnData::F64(Some(1.0))
        );
        assert_eq!(
            read(cell(0x7A, &[], &20000i32.to_le_bytes()))
                .await
                .unwrap(),
            ColumnData::F64(Some(2.0))
        );
    }

    #[tokio::test]
    async fn a_datetime_and_a_small_datetime_keep_their_parts() {
        let mut datetime = 5i32.to_le_bytes().to_vec();
        datetime.extend_from_slice(&300u32.to_le_bytes());
        assert_eq!(
            read(cell(0x3D, &[], &datetime)).await.unwrap(),
            ColumnData::DateTime(Some(crate::tds::time::DateTime::new(5, 300)))
        );

        let mut small = 4u16.to_le_bytes().to_vec();
        small.extend_from_slice(&30u16.to_le_bytes());
        assert_eq!(
            read(cell(0x3A, &[], &small)).await.unwrap(),
            ColumnData::SmallDateTime(Some(crate::tds::time::SmallDateTime::new(4, 30)))
        );
    }

    #[tokio::test]
    async fn a_guid_takes_the_order_of_the_wire() {
        let mut bytes = [0u8; 16];
        for (index, item) in bytes.iter_mut().enumerate() {
            *item = index as u8;
        }
        let mut expected = bytes;
        guid::reorder_bytes(&mut expected);

        assert_eq!(
            read(cell(0x24, &[], &bytes)).await.unwrap(),
            ColumnData::Guid(Some(Uuid::from_bytes(expected)))
        );
    }

    #[tokio::test]
    async fn a_decimal_keeps_its_sign_and_its_scale() {
        let mut value = vec![1];
        value.extend_from_slice(&12345u32.to_le_bytes());
        let data = read(cell(0x6C, &[18, 2], &value)).await.unwrap();
        let ColumnData::Numeric(Some(number)) = data else {
            panic!("the value is not a decimal");
        };
        assert_eq!(number.value(), 12345);
        assert_eq!(number.scale(), 2);

        let mut negative = vec![0];
        negative.extend_from_slice(&500u32.to_le_bytes());
        let data = read(cell(0x6A, &[10, 3], &negative)).await.unwrap();
        let ColumnData::Numeric(Some(number)) = data else {
            panic!("the value is not a decimal");
        };
        assert_eq!(number.value(), -500);
        assert_eq!(number.scale(), 3);
    }

    #[tokio::test]
    async fn a_date_and_a_time_keep_their_counts() {
        assert_eq!(
            read(cell(0x28, &[], &[10, 0, 0])).await.unwrap(),
            ColumnData::Date(Some(crate::tds::time::Date::new(10)))
        );
        assert_eq!(
            read(cell(0x29, &[0], &[1, 0, 0])).await.unwrap(),
            ColumnData::Time(Some(crate::tds::time::Time::new(1, 0)))
        );

        let datetime2 = [1, 0, 0, 10, 0, 0];
        assert_eq!(
            read(cell(0x2A, &[0], &datetime2)).await.unwrap(),
            ColumnData::DateTime2(Some(crate::tds::time::DateTime2::new(
                crate::tds::time::Date::new(10),
                crate::tds::time::Time::new(1, 0),
            )))
        );

        let offset = [1, 0, 0, 10, 0, 0, 60, 0];
        assert_eq!(
            read(cell(0x2B, &[0], &offset)).await.unwrap(),
            ColumnData::DateTimeOffset(Some(crate::tds::time::DateTimeOffset::new(
                crate::tds::time::DateTime2::new(
                    crate::tds::time::Date::new(10),
                    crate::tds::time::Time::new(1, 0),
                ),
                60,
            )))
        );
    }

    #[tokio::test]
    async fn binary_and_text_keep_their_bytes() {
        assert_eq!(
            read(cell(0xA5, &8000u16.to_le_bytes(), &[1, 2, 3]))
                .await
                .unwrap(),
            ColumnData::Binary(Some(Cow::from(vec![1, 2, 3])))
        );

        let utf16: Vec<u8> = "hi".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(
            read(cell(0xE7, &text_props(0, 0), &utf16)).await.unwrap(),
            ColumnData::String(Some("hi".into()))
        );

        assert_eq!(
            read(cell(0xA7, &text_props(0x0409, 0), b"hi"))
                .await
                .unwrap(),
            ColumnData::String(Some("hi".into()))
        );
    }

    #[tokio::test]
    async fn a_header_that_does_not_fit_stops_the_read() {
        let mut short = BytesMut::new();
        short.put_u32_le(1);
        assert!(error(short).await.contains("a length of 1 is too short"));

        let mut props = BytesMut::new();
        props.put_u32_le(3);
        props.put_u8(0x38);
        props.put_u8(7);
        assert!(error(props)
            .await
            .contains("a length of 3 does not hold 7 property bytes"));
    }

    #[tokio::test]
    async fn a_base_type_that_the_reader_cannot_take_stops_the_read() {
        assert!(error(cell(0x99, &[], &[])).await.contains("is not known"));
        assert!(error(cell(0x1F, &[], &[])).await.contains("null"));
        assert!(error(cell(0xF1, &[], &[])).await.contains("not supported"));
    }

    #[tokio::test]
    async fn a_length_or_a_count_that_does_not_match_stops_the_read() {
        assert!(error(cell(0x38, &[], &[1, 2]))
            .await
            .contains("asks for 4 bytes and the value holds 2"));
        assert!(error(cell(0x24, &[0], &[0u8; 16]))
            .await
            .contains("asks for 0 property bytes and the value holds 1"));
    }

    #[tokio::test]
    async fn a_value_that_is_cut_short_stops_the_read() {
        assert!(error(cell(0x2A, &[0], &[1, 0]))
            .await
            .contains("a datetime2 of 2 bytes is too short"));
        assert!(error(cell(0x2B, &[0], &[1, 0, 0, 10]))
            .await
            .contains("a datetimeoffset of 4 bytes is too short"));
        assert!(error(cell(0x6C, &[18, 2], &[]))
            .await
            .contains("a decimal holds no digits"));
    }

    #[tokio::test]
    async fn a_decimal_that_the_target_type_cannot_hold_stops_the_read() {
        let mut wide = vec![1];
        wide.extend_from_slice(&[0xFF; 17]);
        assert!(error(cell(0x6C, &[38, 0], &wide))
            .await
            .contains("a decimal of 17 bytes is too wide"));

        let mut full = vec![1];
        full.extend_from_slice(&[0xFF; 16]);
        assert!(error(cell(0x6C, &[38, 0], &full))
            .await
            .contains("wider than the target type"));

        let mut sign = vec![2];
        sign.extend_from_slice(&1u32.to_le_bytes());
        assert!(error(cell(0x6C, &[18, 0], &sign))
            .await
            .contains("a sign of 2 is not valid"));
    }

    #[tokio::test]
    async fn a_text_that_holds_a_half_unit_stops_the_read() {
        assert!(error(cell(0xE7, &text_props(0, 0), &[1, 2, 3]))
            .await
            .contains("holds a half unit"));
    }

    #[tokio::test]
    async fn a_collation_that_the_reader_does_not_know_stops_the_read() {
        assert!(error(cell(0xA7, &text_props(0xFFFF, 0), b"hi"))
            .await
            .contains("encoding"));
    }
}
