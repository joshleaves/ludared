//! Helpers shared by the Nintendo 3DS formats.
//!
//! Reading a 3DS image means pulling bounds-checked little-endian fields out of a
//! fixed header. None of that belongs to a single container, so it lives here.
//!
//! Every reader is fallible: it names the offset it could not read when the
//! buffer is shorter than the structure it should hold.

use crate::codecs::errors::CodecError;

/// The `N` bytes at `offset`, or an error naming the offset.
pub(crate) fn field<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N], CodecError> {
  data
    .get(offset..)
    .and_then(|rest| rest.first_chunk::<N>())
    .copied()
    .ok_or_else(|| {
      CodecError::Message(format!(
        "3DS structure is truncated: the {N}-byte field at {offset:#x} does not fit"
      ))
    })
}

/// The little-endian `u32` at `offset`, or an error naming the offset.
pub(crate) fn u32_le(data: &[u8], offset: usize) -> Result<u32, CodecError> {
  field(data, offset).map(u32::from_le_bytes)
}

/// The little-endian `u64` at `offset`, or an error naming the offset.
pub(crate) fn u64_le(data: &[u8], offset: usize) -> Result<u64, CodecError> {
  field(data, offset).map(u64::from_le_bytes)
}
