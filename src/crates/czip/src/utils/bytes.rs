use crate::CZipError;
use tracing::trace;

pub fn read_u32(bytes: &[u8], cursor: &mut usize, label: &'static str) -> crate::Result<u32> {
    let raw = read_exact(bytes, cursor, 4, label)?;
    let array = raw.try_into().expect("slice of length 4");
    let value = u32::from_le_bytes(array);
    trace!(
        label = label,
        value,
        offset = *cursor,
        "Read u32 from buffer"
    );
    Ok(value)
}

pub fn read_exact<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    len: usize,
    label: &'static str,
) -> crate::Result<&'a [u8]> {
    let end = cursor
        .checked_add(len)
        .ok_or(CZipError::UnexpectedEof(label))?;

    if end > bytes.len() {
        return Err(CZipError::UnexpectedEof(label));
    }

    let slice = &bytes[*cursor..end];
    *cursor = end;
    trace!(label = label, start = end - len, end, "Read {} bytes", len);
    Ok(slice)
}
