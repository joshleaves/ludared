//! Bounds-checked byte-field readers shared by codecs.

#[cfg(test)]
mod tests;

use crate::codecs::errors::CodecError;

/// The `N` bytes at `offset`, or an error naming the offset.
pub(crate) fn read_field<const N: usize>(
  data: &[u8],
  offset: usize,
) -> Result<[u8; N], CodecError> {
  data
    .get(offset..)
    .and_then(|rest| rest.first_chunk::<N>())
    .copied()
    .ok_or_else(|| {
      CodecError::Message(format!(
        "Structure is truncated: the {N}-byte field at {offset:#x} does not fit"
      ))
    })
}

/// Reads the little-endian `u16` at `offset`.
pub(crate) fn read_u16_le(data: &[u8], offset: usize) -> Result<u16, CodecError> {
  read_field(data, offset).map(u16::from_le_bytes)
}

/// Reads the little-endian `u32` at `offset`.
pub(crate) fn read_u32_le(data: &[u8], offset: usize) -> Result<u32, CodecError> {
  read_field(data, offset).map(u32::from_le_bytes)
}

/// Reads the little-endian `u64` at `offset`.
pub(crate) fn read_u64_le(data: &[u8], offset: usize) -> Result<u64, CodecError> {
  read_field(data, offset).map(u64::from_le_bytes)
}

/// Reads the big-endian `u16` at `offset`.
pub(crate) fn read_u16_be(data: &[u8], offset: usize) -> Result<u16, CodecError> {
  read_field(data, offset).map(u16::from_be_bytes)
}

/// Reads the big-endian `u32` at `offset`.
pub(crate) fn read_u32_be(data: &[u8], offset: usize) -> Result<u32, CodecError> {
  read_field(data, offset).map(u32::from_be_bytes)
}

/// Reads the big-endian `u64` at `offset`.
pub(crate) fn read_u64_be(data: &[u8], offset: usize) -> Result<u64, CodecError> {
  read_field(data, offset).map(u64::from_be_bytes)
}
