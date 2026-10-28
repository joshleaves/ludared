//! Helpers shared by Nintendo 3DS formats.

use crate::codecs::errors::CodecError;

/// Decodes a metadata hex string that must hold exactly `len` bytes.
pub(crate) fn hex_field(text: &str, what: &str, len: usize) -> Result<Vec<u8>, CodecError> {
  let bytes = hex::decode(text).map_err(|err| {
    CodecError::Message(format!(
      "3DS metadata {what} is not valid hexadecimal: {err}"
    ))
  })?;

  if bytes.len() != len {
    return Err(CodecError::Message(format!(
      "3DS metadata {what} is {} bytes, expected {len}",
      bytes.len()
    )));
  }

  Ok(bytes)
}

/// `value` rounded up to a multiple of `align`, or `None` on overflow.
pub(crate) fn align_up(value: u64, align: u64) -> Option<u64> {
  value
    .checked_add(align.checked_sub(1)?)?
    .checked_div(align)?
    .checked_mul(align)
}
