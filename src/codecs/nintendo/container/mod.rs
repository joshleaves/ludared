//! Nintendo archive container formats.

pub(crate) mod darc;
pub(crate) mod sarc;

use crate::codecs::common::bytes::read_field;
use crate::codecs::errors::CodecError;
use serde::Deserialize;
use serde::Serialize;

/// Every file artifact of these codecs lives under this prefix, leaving the
/// top level free for the metadata artifact.
pub(crate) const FILES_PREFIX: &str = "files/";

/// Byte order of a Nintendo archive, selected by its BOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Endian {
  Little,
  Big,
}

impl Endian {
  /// Detects the byte order from the two-byte marker at `offset`.
  ///
  /// The format stores the `0xFEFF` value in its own byte order, so a
  /// little-endian archive has the bytes `FF FE` and a big-endian one has
  /// `FE FF`.
  pub(crate) fn from_bom(data: &[u8], offset: usize) -> Result<Self, CodecError> {
    match read_field::<2>(data, offset)? {
      [0xFF, 0xFE] => Ok(Self::Little),
      [0xFE, 0xFF] => Ok(Self::Big),
      bytes => Err(CodecError::Message(format!(
        "Archive has an invalid byte-order marker {bytes:02X?} at {offset:#x}"
      ))),
    }
  }

  pub(crate) fn read_u16(self, data: &[u8], offset: usize) -> Result<u16, CodecError> {
    let bytes = read_field::<2>(data, offset)?;
    Ok(match self {
      Self::Little => u16::from_le_bytes(bytes),
      Self::Big => u16::from_be_bytes(bytes),
    })
  }

  pub(crate) fn read_u32(self, data: &[u8], offset: usize) -> Result<u32, CodecError> {
    let bytes = read_field::<4>(data, offset)?;
    Ok(match self {
      Self::Little => u32::from_le_bytes(bytes),
      Self::Big => u32::from_be_bytes(bytes),
    })
  }

  pub(crate) fn u16_bytes(self, value: u16) -> [u8; 2] {
    match self {
      Self::Little => value.to_le_bytes(),
      Self::Big => value.to_be_bytes(),
    }
  }

  pub(crate) fn u32_bytes(self, value: u32) -> [u8; 4] {
    match self {
      Self::Little => value.to_le_bytes(),
      Self::Big => value.to_be_bytes(),
    }
  }

  /// Decodes UTF-16 bytes in this byte order. The caller checks the length.
  pub(crate) fn utf16_units(self, raw: &[u8]) -> Vec<u16> {
    raw
      .as_chunks::<2>()
      .0
      .iter()
      .map(|pair| match self {
        Self::Little => u16::from_le_bytes(*pair),
        Self::Big => u16::from_be_bytes(*pair),
      })
      .collect()
  }

  pub(crate) fn utf16_bytes(self, units: impl IntoIterator<Item = u16>) -> Vec<u8> {
    units
      .into_iter()
      .flat_map(|unit| self.u16_bytes(unit))
      .collect()
  }
}

/// A range of non-zero bytes inside an archive's data area.
///
/// Archives pad file data with zeros; recording only the non-zero ranges keeps
/// an unmodified rebuild byte-exact without duplicating the file contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gap {
  /// Absolute offset within the archive.
  pub(crate) offset: u32,
  /// The bytes at `offset`, hex-encoded.
  pub(crate) data: String,
}

/// Decodes a metadata hex string with a context-specific error message.
pub(crate) fn decode_hex(text: &str, what: &str) -> Result<Vec<u8>, CodecError> {
  hex::decode(text).map_err(|err| {
    CodecError::Message(format!(
      "Archive metadata {what} is not valid hexadecimal: {err}"
    ))
  })
}

/// Collects the non-zero bytes outside `ranges` in `data[start..end]`.
///
/// `ranges` is sorted in place. Overlapping ranges are the caller's problem;
/// this function only reports what the gaps contain.
pub(crate) fn collect_gaps(
  data: &[u8],
  start: usize,
  end: usize,
  ranges: &mut [(usize, usize)],
) -> Result<Vec<Gap>, CodecError> {
  ranges.sort_unstable();
  let mut gaps = Vec::new();
  let mut cursor = start;
  for &(range_start, range_end) in ranges.iter() {
    if range_start > cursor {
      push_gap(&mut gaps, data, cursor, range_start)?;
    }
    cursor = cursor.max(range_end);
  }
  if cursor < end {
    push_gap(&mut gaps, data, cursor, end)?;
  }
  Ok(gaps)
}

fn push_gap(gaps: &mut Vec<Gap>, data: &[u8], start: usize, end: usize) -> Result<(), CodecError> {
  let Some(bytes) = data.get(start..end) else {
    return Ok(());
  };
  if bytes.iter().any(|&byte| byte != 0) {
    // A gap offset is an absolute archive offset, and the format stores those
    // in 32 bits.
    let offset = u32::try_from(start).map_err(|_| {
      CodecError::Message(format!(
        "Archive gap at {start:#x} does not fit a 32-bit offset"
      ))
    })?;
    gaps.push(Gap {
      offset,
      data: hex::encode(bytes),
    });
  }
  Ok(())
}

/// Writes a list of gaps into `buffer`, which represents `buffer_offset..`.
pub(crate) fn write_gaps(
  buffer: &mut [u8],
  buffer_offset: u32,
  gaps: &[Gap],
) -> Result<(), CodecError> {
  for gap in gaps {
    let bytes = decode_hex(&gap.data, "gap")?;
    let start = gap.offset.checked_sub(buffer_offset).ok_or_else(|| {
      CodecError::Message(format!(
        "Archive metadata gap at {:#x} starts before the data area at {buffer_offset:#x}",
        gap.offset
      ))
    })? as usize;
    let end = start
      .checked_add(bytes.len())
      .ok_or_else(|| CodecError::Message("Archive metadata gap range overflows".to_owned()))?;
    let target = buffer.get_mut(start..end).ok_or_else(|| {
      CodecError::Message(format!(
        "Archive metadata gap at {:#x} ({} bytes) exceeds the data area",
        gap.offset,
        bytes.len()
      ))
    })?;
    target.copy_from_slice(&bytes);
  }
  Ok(())
}

/// Whether a metadata field is zero, for `skip_serializing_if`.
#[expect(
  clippy::trivially_copy_pass_by_ref,
  reason = "serde calls skip_serializing_if predicates with a reference"
)]
pub(crate) fn is_zero_u64(value: &u64) -> bool {
  *value == 0
}

/// Splits the bytes after an archive's declared size into their metadata form.
///
/// A tail of zeros is recorded as its length alone, so a padded archive does
/// not carry a hex string of zeros; a tail with any non-zero byte keeps its
/// exact bytes (including the zeros around them) and no length, so mixed
/// padding still round-trips.
pub(crate) fn split_trailing(bytes: &[u8]) -> (String, u64) {
  if bytes.iter().all(|&byte| byte == 0) {
    (String::new(), bytes.len() as u64)
  } else {
    (hex::encode(bytes), 0)
  }
}

/// Materializes the bytes after an archive's declared size from metadata.
///
/// Metadata written before `trailing_zeros` existed has neither the field nor
/// the hex string, and still means an archive with no tail.
pub(crate) fn join_trailing(trailing: &str, trailing_zeros: u64) -> Result<Vec<u8>, CodecError> {
  let bytes = decode_hex(trailing, "trailing bytes")?;
  if bytes.is_empty() {
    let length = usize::try_from(trailing_zeros).map_err(|_| {
      CodecError::Message(
        "Archive metadata trailing zero length does not fit this platform".to_owned(),
      )
    })?;
    return Ok(vec![0; length]);
  }
  if trailing_zeros != 0 {
    return Err(CodecError::Message(format!(
      "Archive metadata records both {trailing_zeros} trailing zero bytes and {} bytes of trailing data",
      bytes.len()
    )));
  }
  Ok(bytes)
}

/// Refuses empty components, absolute paths, separators other than `/`, and
/// the `.`/`..` components the framework considers invalid.
pub(crate) fn validate_relative_path(path: &str, what: &str) -> Result<(), CodecError> {
  if path.is_empty() {
    return Err(CodecError::Message(format!("{what} path is empty")));
  }
  if path.starts_with('/') || path.ends_with('/') {
    return Err(CodecError::Message(format!(
      "{what} path {path:?} must not start or end with '/'"
    )));
  }
  if path.contains('\\') {
    return Err(CodecError::Message(format!(
      "{what} path {path:?} contains a backslash"
    )));
  }
  if path
    .split('/')
    .any(|component| component.is_empty() || component == "." || component == "..")
  {
    return Err(CodecError::Message(format!(
      "{what} path {path:?} contains an empty, '.' or '..' component"
    )));
  }
  Ok(())
}

/// Refuses duplicate artifact paths and paths that are an ancestor of another.
pub(crate) fn validate_file_paths<'a>(
  paths: impl IntoIterator<Item = &'a str>,
) -> Result<(), CodecError> {
  let mut artifacts = std::collections::BTreeSet::new();
  for path in paths {
    let artifact = format!("{FILES_PREFIX}{path}");
    if !artifacts.insert(artifact.clone()) {
      return Err(CodecError::DuplicateArtifact(artifact));
    }
  }
  for artifact in &artifacts {
    let mut ancestor = artifact.as_str();
    while let Some((parent, _)) = ancestor.rsplit_once('/') {
      if artifacts.contains(parent) {
        return Err(CodecError::Message(format!(
          "Archive artifact paths overlap: {parent:?} is a parent of {artifact:?}"
        )));
      }
      ancestor = parent;
    }
  }
  Ok(())
}
