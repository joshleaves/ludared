//! Format-independent byte addressing and byte serialization for CD images.

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use serde::de::Error as _;

/// The size of a logical CD-ROM data sector in bytes.
pub(crate) const BLOCK_SIZE: usize = 2048;

/// A half-open range of bytes in an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ByteRange {
  pub start: u64,
  pub end: u64,
}

impl ByteRange {
  /// Builds a range if its end can be represented.
  pub(crate) fn new(offset: u64, length: u64) -> Option<Self> {
    Some(Self {
      start: offset,
      end: offset.checked_add(length)?,
    })
  }

  /// Whether two half-open byte ranges share any bytes.
  pub(crate) fn overlaps(self, other: Self) -> bool {
    self.start < other.end && other.start < self.end
  }
}

/// A byte offset for a logical CD-ROM sector, if it fits in 64 bits.
pub(crate) fn block_offset(block: u64) -> Option<u64> {
  block.checked_mul(BLOCK_SIZE as u64)
}

/// Bytes serialized as lowercase hexadecimal in a metadata artifact.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Hex(pub(crate) Vec<u8>);

impl Hex {
  pub(crate) fn is_empty(&self) -> bool {
    self.0.is_empty()
  }

  pub(crate) fn as_bytes(&self) -> &[u8] {
    &self.0
  }

  pub(crate) fn len(&self) -> usize {
    self.0.len()
  }
}

impl From<Vec<u8>> for Hex {
  fn from(bytes: Vec<u8>) -> Self {
    Self(bytes)
  }
}

impl From<&[u8]> for Hex {
  fn from(bytes: &[u8]) -> Self {
    Self(bytes.to_vec())
  }
}

impl Serialize for Hex {
  fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&hex::encode(&self.0))
  }
}

impl<'de> Deserialize<'de> for Hex {
  fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    let text = String::deserialize(deserializer)?;
    hex::decode(&text).map(Self).map_err(D::Error::custom)
  }
}
