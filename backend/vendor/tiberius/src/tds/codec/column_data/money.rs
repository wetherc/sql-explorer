use crate::{error::Error, sql_read_bytes::SqlReadBytes, tds::Numeric, ColumnData};

pub(crate) async fn decode<R>(src: &mut R, len: u8) -> crate::Result<ColumnData<'static>>
where
    R: SqlReadBytes + Unpin,
{
    // A money value is a whole number of ten-thousandths. A float holds 15
    // or 16 significant digits, and a money value holds up to 19, so the
    // value comes out as a decimal with a scale of four.
    let res = match len {
        0 => ColumnData::Numeric(None),
        4 => ColumnData::Numeric(Some(Numeric::new_with_scale(
            src.read_i32_le().await? as i128,
            4,
        ))),
        8 => ColumnData::Numeric(Some({
            let high = src.read_i32_le().await? as i64;
            let low = src.read_u32_le().await? as i64;

            Numeric::new_with_scale(((high << 32) | low) as i128, 4)
        })),
        _ => {
            return Err(Error::Protocol(
                format!("money: length of {} is invalid", len).into(),
            ))
        }
    };

    Ok(res)
}
