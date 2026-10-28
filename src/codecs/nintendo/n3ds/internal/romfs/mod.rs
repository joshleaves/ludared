//! Nintendo 3DS RomFS (Read-Only Filesystem) codec.
//!
//! Extracts the file tree of a RomFS image and rebuilds it. The rebuild
//! recomputes the directory/file metadata tables, the hash tables and the IVFC
//! hash tree (levels 0-2) from the artifact bytes, following the makerom
//! layout (`makerom/src/romfs_gen.c` in [Project CTR]); the original file data
//! offsets are preserved while they still fit the artifacts, so an unmodified
//! rebuild stays byte-identical.
//!
//! Format references:
//! - <https://www.3dbrew.org/wiki/RomFS>
//! - [Project CTR](https://github.com/3DSGuy/Project_CTR) `makerom/src/romfs_gen.c`
//!
//! Attribution: the tree layout, hash-table and hashing algorithms are a port
//! of 3DSGuy's makerom RomFS generation code (see ATTRIBUTIONS.md).

use crate::codecs::common::bytes::{
  read_field as field, read_u32_le as u32_le, read_u64_le as u64_le,
};
use crate::codecs::common::hash::sha256;
use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::n3ds::common::{align_up, hex_field};
use crate::codecs::{Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact, LudaredCodecArgs};
use log::*;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

const CODEC_ID: &str = "std/nintendo/3ds/internal/romfs";
const CODEC_NAME: &str = "Nintendo 3DS RomFS";
const CODEC_DESC: &str = r#"Nintendo 3DS RomFS (Read-Only Filesystem)

Extracts a RomFS image into:
- RomFS.json: IVFC header fields, hash-table sizes and the file tree with the
  original entry order and data offsets, so an unmodified rebuild stays
  byte-identical.
- One artifact per file, under files/ with its full path.

Rebuilding recomputes the metadata tables, the name hash tables and the whole
IVFC hash tree from the artifact bytes, using the makerom layout rules. The
original file data offsets are reused while they still fit the artifacts; once
a file changes size the data area is laid out again, sequentially on 16-byte
boundaries. The rebuild is byte-exact for makerom-built images.

Entry names are UTF-16; names that are not valid UTF-16 keep their raw bytes
in RomFS.json. Edited trees keep working: links, tables and hashes follow the
metadata.

No arguments.
"#;

const IVFC_MAGIC: &[u8; 4] = b"IVFC";
const IVFC_HEADER_SIZE: usize = 0x5C;
const LEVEL0_HEADER_SIZE: u64 = 0x60;
const MASTER_HASH_SIZE_OFFSET: usize = 0x08;
const LEVEL_HEADERS_OFFSET: usize = 0x0C;
const LEVEL_HEADER_SIZE: usize = 0x18;
const LEVEL_COUNT: usize = 3;
const RESERVED_TAIL_OFFSET: usize = 0x54;
const RESERVED_TAIL_SIZE: usize = 0x04;
const OPTIONAL_INFO_SIZE_OFFSET: usize = 0x58;
const L3_HEADER_SIZE: usize = 0x28;
const DIR_ENTRY_FIXED: usize = 0x18;
const FILE_ENTRY_FIXED: usize = 0x20;
const UNUSED_ENTRY: u32 = 0xFFFFFFFF;
const DATA_ALIGN: u64 = 0x10;
const HASH_SIZE_BYTES: usize = 0x20;
const METADATA_NAME: &str = "RomFS.json";
const FILES_PREFIX: &str = "files/";

pub(crate) struct N3dsRomFs;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct N3dsRomFsCodecArgs {}

impl LudaredCodecArgs for N3dsRomFsCodecArgs {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LevelMetadata {
  logical_offset: u64,
  hash_data_size: u64,
  block_size_log2: u32,
  reserved: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirMetadata {
  /// Full path of the parent directory ("" for the root's children).
  parent: String,
  /// UTF-8 view of the UTF-16 name ("" for the root).
  name: String,
  /// Raw UTF-16 bytes when they do not round-trip through UTF-8.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  name_raw: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileMetadata {
  parent: String,
  name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  name_raw: Option<String>,
  /// Offset within the file data area, as decoded.
  data_offset: u64,
  size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RomfsMetadata {
  magic_id: u32,
  master_hash_size: u32,
  optional_info_size: u32,
  /// Hex of the 8 reserved bytes at 0x50.
  reserved_tail: String,
  /// Levels 1, 2 and 3, in that order.
  levels: [LevelMetadata; LEVEL_COUNT],
  /// Hash bucket counts; absent means the makerom rule over the entry count.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  dir_hash_buckets: Option<u32>,
  /// See `dir_hash_buckets`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  file_hash_buckets: Option<u32>,
  /// Directories in physical table order; the first entry is the root.
  directories: Vec<DirMetadata>,
  /// Files in physical table order.
  files: Vec<FileMetadata>,
}

impl RomfsMetadata {
  fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the RomFS metadata: {err}")))
  }

  fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the RomFS metadata artifact: {err}"))
    })
  }
}

/// A parsed directory or file metadata entry, before tree interpretation.
struct RawEntry {
  parent: u32,
  name: Vec<u8>, // UTF-16LE, unpadded
  // File data location (0 for directories).
  data_offset: u64,
  data_size: u64,
}

fn align4(value: u64) -> u64 {
  value.div_ceil(4) * 4
}

fn dir_entry_size(name_len: usize) -> u64 {
  DIR_ENTRY_FIXED as u64 + align4(name_len as u64)
}

fn file_entry_size(name_len: usize) -> u64 {
  FILE_ENTRY_FIXED as u64 + align4(name_len as u64)
}

/// Reads every entry of a metadata table in physical order.
fn parse_dir_table(table: &[u8]) -> Result<Vec<RawEntry>, CodecError> {
  let mut entries = Vec::new();
  let mut offset = 0usize;
  while offset < table.len() {
    let parent = u32_le(table, offset)?;
    // Sibling, child, first-file and hash links are rebuilt by the encoder;
    // reading them here only bounds-checks the fixed part of the entry.
    let _ = u32_le(table, offset + 0x10)?;
    let name_size = u32_le(table, offset + 0x14)? as usize;
    let name = table
      .get(offset + DIR_ENTRY_FIXED..offset + DIR_ENTRY_FIXED + name_size)
      .ok_or_else(|| {
        CodecError::Message(format!("RomFS directory entry at {offset:#x} is truncated"))
      })?
      .to_vec();
    entries.push(RawEntry {
      parent,
      name,
      data_offset: 0,
      data_size: 0,
    });
    offset += dir_entry_size(name_size) as usize;
  }
  if offset != table.len() {
    return Err(CodecError::Message(
      "RomFS directory metadata table ends mid-entry".to_owned(),
    ));
  }
  Ok(entries)
}

fn parse_file_table(table: &[u8]) -> Result<Vec<RawEntry>, CodecError> {
  let mut entries = Vec::new();
  let mut offset = 0usize;
  while offset < table.len() {
    let parent = u32_le(table, offset)?;
    // Sibling and hash links are rebuilt by the encoder; this read only
    // bounds-checks the fixed part of the entry.
    let _ = u32_le(table, offset + 0x4)?;
    let data_offset = u64_le(table, offset + 0x8)?;
    let data_size = u64_le(table, offset + 0x10)?;
    let name_size = u32_le(table, offset + 0x1C)? as usize;
    let name = table
      .get(offset + FILE_ENTRY_FIXED..offset + FILE_ENTRY_FIXED + name_size)
      .ok_or_else(|| CodecError::Message(format!("RomFS file entry at {offset:#x} is truncated")))?
      .to_vec();
    entries.push(RawEntry {
      parent,
      name,
      data_offset,
      data_size,
    });
    offset += file_entry_size(name_size) as usize;
  }
  if offset != table.len() {
    return Err(CodecError::Message(
      "RomFS file metadata table ends mid-entry".to_owned(),
    ));
  }
  Ok(entries)
}

/// UTF-16LE bytes to (display name, raw hex when not round-trippable).
fn decode_name(raw: &[u8], table: &str) -> Result<(String, Option<String>), CodecError> {
  if !raw.len().is_multiple_of(2) {
    return Err(CodecError::Message(format!(
      "RomFS {table} entry has an odd-length name of {} bytes",
      raw.len()
    )));
  }
  let units: Vec<u16> = raw
    .as_chunks::<2>()
    .0
    .iter()
    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
    .collect();
  match String::from_utf16(&units) {
    Ok(name)
      if name
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .eq(raw.iter().copied()) =>
    {
      Ok((name, None))
    }
    // Unpaired surrogates or non-round-trippable text: keep the raw bytes and
    // use a path-safe placeholder that is reversible through `name_raw`.
    _ => Ok((
      format!("_romfs_raw_{}", hex::encode(raw)),
      Some(hex::encode(raw)),
    )),
  }
}

fn validate_file_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> Result<(), CodecError> {
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
          "RomFS artifact paths overlap: {parent:?} is a parent of {artifact:?}"
        )));
      }
      ancestor = parent;
    }
  }
  Ok(())
}

fn encode_name(entry_name: &str, name_raw: &Option<String>) -> Result<Vec<u8>, CodecError> {
  match name_raw {
    Some(raw) => {
      let bytes = hex::decode(raw).map_err(|err| {
        CodecError::Message(format!(
          "RomFS metadata raw name of {entry_name:?} is not valid hexadecimal: {err}"
        ))
      })?;
      if bytes.len() % 2 != 0 {
        return Err(CodecError::Message(format!(
          "RomFS metadata raw name of {entry_name:?} has an odd length"
        )));
      }
      Ok(bytes)
    }
    None => Ok(
      entry_name
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect(),
    ),
  }
}

/// The makerom bucket-count rule (romfs_gen.c GetHashTableCount).
fn hash_table_count(entries: u32) -> u32 {
  if entries < 3 {
    return 3;
  }
  let mut count = entries;
  if count < 19 {
    count |= 1;
  } else {
    while count.is_multiple_of(2)
      || count.is_multiple_of(3)
      || count.is_multiple_of(5)
      || count.is_multiple_of(7)
      || count.is_multiple_of(11)
      || count.is_multiple_of(13)
      || count.is_multiple_of(17)
    {
      count += 1;
    }
  }
  count
}

/// The makerom path hash (romfs_gen.c CalcPathHash).
fn calc_path_hash(parent_offset: u32, name: &[u8]) -> u32 {
  let mut hash = parent_offset ^ 123456789;
  for pair in name.as_chunks::<2>().0 {
    hash = hash.rotate_right(5);
    hash ^= u32::from(u16::from_le_bytes([pair[0], pair[1]]));
  }
  hash
}

struct IvfcHeader {
  magic_id: u32,
  master_hash_size: u32,
  levels: [LevelMetadata; LEVEL_COUNT],
  reserved_tail: [u8; RESERVED_TAIL_SIZE],
  optional_info_size: u32,
}

fn parse_ivfc(data: &[u8]) -> Result<IvfcHeader, CodecError> {
  let header = data.get(..IVFC_HEADER_SIZE).ok_or_else(|| {
    CodecError::Message(format!(
      "RomFS header is truncated: {} bytes (Minimum: {IVFC_HEADER_SIZE})",
      data.len()
    ))
  })?;
  if header[..4] != IVFC_MAGIC[..] {
    return Err(CodecError::Message(
      "Not a RomFS image: the 'IVFC' magic is missing".to_owned(),
    ));
  }
  let mut levels = [LevelMetadata {
    logical_offset: 0,
    hash_data_size: 0,
    block_size_log2: 0,
    reserved: 0,
  }; LEVEL_COUNT];
  for (index, level) in levels.iter_mut().enumerate() {
    let base = LEVEL_HEADERS_OFFSET + index * LEVEL_HEADER_SIZE;
    *level = LevelMetadata {
      logical_offset: u64_le(header, base)?,
      hash_data_size: u64_le(header, base + 0x8)?,
      block_size_log2: u32_le(header, base + 0x10)?,
      reserved: u32_le(header, base + 0x14)?,
    };
  }
  Ok(IvfcHeader {
    magic_id: u32_le(header, 0x4)?,
    master_hash_size: u32_le(header, MASTER_HASH_SIZE_OFFSET)?,
    levels,
    reserved_tail: field(header, RESERVED_TAIL_OFFSET)?,
    optional_info_size: u32_le(header, OPTIONAL_INFO_SIZE_OFFSET)?,
  })
}

impl IvfcHeader {
  /// The logical offsets link levels in the hash tree; the file stores them
  /// physically in the order Level 3, Level 1, Level 2.
  fn level(&self, index: usize) -> Result<std::ops::Range<usize>, CodecError> {
    let block_size = |index: usize| -> Result<u64, CodecError> {
      let log2 = self.levels[index].block_size_log2;
      if log2 >= 32 {
        return Err(CodecError::Message(format!(
          "RomFS level {} block size 2^{log2} is implausible",
          index + 1
        )));
      }
      Ok(1u64 << log2)
    };
    let level3_start = align_up(
      (IVFC_HEADER_SIZE as u64)
        .div_ceil(0x10)
        .checked_mul(0x10)
        .and_then(|start| start.checked_add(u64::from(self.master_hash_size)))
        .ok_or_else(|| CodecError::Message("RomFS master hash range overflows".to_owned()))?,
      block_size(2)?,
    )
    .ok_or_else(|| CodecError::Message("RomFS level 3 offset overflows".to_owned()))?;
    let level3_end = level3_start
      .checked_add(self.levels[2].hash_data_size)
      .ok_or_else(|| CodecError::Message("RomFS level 3 range overflows".to_owned()))?;
    let level1_start = align_up(level3_end, block_size(2)?)
      .ok_or_else(|| CodecError::Message("RomFS level 1 offset overflows".to_owned()))?;
    let level1_end = level1_start
      .checked_add(self.levels[0].hash_data_size)
      .ok_or_else(|| CodecError::Message("RomFS level 1 range overflows".to_owned()))?;
    let level2_start = align_up(level1_end, block_size(0)?)
      .ok_or_else(|| CodecError::Message("RomFS level 2 offset overflows".to_owned()))?;
    let starts = [
      level1_start,
      level2_start,
      level3_start,
    ];
    let start =
      usize::try_from(*starts.get(index).ok_or_else(|| {
        CodecError::Message(format!("RomFS level index {index} is out of range"))
      })?)
      .map_err(|_| {
        CodecError::Message(format!(
          "RomFS level {} offset is impossibly large",
          index + 1
        ))
      })?;
    let size = usize::try_from(self.levels[index].hash_data_size).map_err(|_| {
      CodecError::Message(format!(
        "RomFS level {} size is impossibly large",
        index + 1
      ))
    })?;
    let end = start
      .checked_add(size)
      .ok_or_else(|| CodecError::Message(format!("RomFS level {} range overflows", index + 1)))?;
    Ok(start..end)
  }

  fn logical_layout_matches(&self) -> Result<bool, CodecError> {
    if self.levels[0].logical_offset != 0 {
      return Ok(false);
    }
    let block_size = |index: usize| -> Result<u64, CodecError> {
      let log2 = self.levels[index].block_size_log2;
      if log2 >= 32 {
        return Err(CodecError::Message(format!(
          "RomFS level {} block size 2^{log2} is implausible",
          index + 1
        )));
      }
      Ok(1u64 << log2)
    };
    let expected_level2 = self.levels[0]
      .logical_offset
      .checked_add(self.levels[0].hash_data_size)
      .and_then(|end| align_up(end, block_size(0).ok()?));
    let expected_level2 = expected_level2
      .ok_or_else(|| CodecError::Message("RomFS logical Level 2 offset overflows".to_owned()))?;
    let expected_level3 = self.levels[1]
      .logical_offset
      .checked_add(self.levels[1].hash_data_size)
      .and_then(|end| align_up(end, block_size(1).ok()?));
    let expected_level3 = expected_level3
      .ok_or_else(|| CodecError::Message("RomFS logical Level 3 offset overflows".to_owned()))?;
    Ok(
      self.levels[1].logical_offset == expected_level2
        && self.levels[2].logical_offset == expected_level3,
    )
  }
}

/// Hashes `data` in `block_size`-large blocks, zero-padding the last one.
fn hash_level(data: &[u8], block_size: u64) -> Result<Vec<u8>, CodecError> {
  let block = usize::try_from(block_size)
    .map_err(|_| CodecError::Message("RomFS hash block is impossibly large".to_owned()))?;
  if block == 0 {
    return Err(CodecError::Message(
      "RomFS hash block size is zero".to_owned(),
    ));
  }
  let blocks = data.len().div_ceil(block).max(1);
  let capacity = blocks
    .checked_mul(HASH_SIZE_BYTES)
    .ok_or_else(|| CodecError::Message("RomFS hash level size overflows".to_owned()))?;
  let mut out = Vec::with_capacity(capacity);
  for index in 0..blocks {
    let start = index * block;
    let end = (start + block).min(data.len());
    let mut buffer = vec![0u8; block];
    buffer[..end - start].copy_from_slice(&data[start..end]);
    out.extend_from_slice(&sha256(&buffer));
  }
  Ok(out)
}

impl Codec for N3dsRomFs {
  fn id(&self) -> &str {
    CODEC_ID
  }

  fn name(&self) -> &str {
    CODEC_NAME
  }

  fn description(&self) -> &str {
    CODEC_DESC
  }

  fn kind(&self) -> CodecKind {
    CodecKind::Rebuild
  }

  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    let Ok(header) = parse_ivfc(data) else {
      return Ok(CodecHandlingConfidence::No);
    };
    if header.magic_id != 0x10000
      || header
        .levels
        .iter()
        .any(|level| level.block_size_log2 >= 32)
    {
      return Ok(CodecHandlingConfidence::No);
    }
    // The level 3 region must hold a readable filesystem header.
    let Ok(level3) = header.level(2) else {
      return Ok(CodecHandlingConfidence::No);
    };
    let Some(l3) = data.get(level3.clone()) else {
      return Ok(CodecHandlingConfidence::No);
    };
    if l3.len() < L3_HEADER_SIZE {
      return Ok(CodecHandlingConfidence::No);
    }
    let Ok(header_length) = u32_le(l3, 0) else {
      return Ok(CodecHandlingConfidence::No);
    };
    if header_length as usize != L3_HEADER_SIZE {
      return Ok(CodecHandlingConfidence::No);
    }
    if header.logical_layout_matches()? {
      Ok(CodecHandlingConfidence::Certain)
    } else {
      Ok(CodecHandlingConfidence::Likely)
    }
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("romfs".to_owned())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding Nintendo 3DS RomFS image");
    let _args = N3dsRomFsCodecArgs::from_args(args)?;

    let header = parse_ivfc(data)?;
    let level3_range = header.level(2)?;
    let level3 = data
      .get(level3_range.clone())
      .ok_or_else(|| CodecError::Message("RomFS level 3 region exceeds the image".to_owned()))?;

    let header_length = u32_le(level3, 0)? as usize;
    if header_length != L3_HEADER_SIZE {
      return Err(CodecError::Message(format!(
        "RomFS level 3 header is {header_length} bytes, expected {L3_HEADER_SIZE}"
      )));
    }
    let dir_hash_offset = u32_le(level3, 0x4)? as usize;
    let dir_hash_length = u32_le(level3, 0x8)? as usize;
    let dir_meta_offset = u32_le(level3, 0xC)? as usize;
    let dir_meta_length = u32_le(level3, 0x10)? as usize;
    let file_hash_offset = u32_le(level3, 0x14)? as usize;
    let file_hash_length = u32_le(level3, 0x18)? as usize;
    let file_meta_offset = u32_le(level3, 0x1C)? as usize;
    let file_meta_length = u32_le(level3, 0x20)? as usize;
    let data_offset = u32_le(level3, 0x24)? as usize;

    let section = |offset: usize, length: usize, what: &str| {
      let end = offset.checked_add(length).ok_or_else(|| {
        CodecError::Message(format!("RomFS {what} range overflows the level 3 region"))
      })?;
      level3
        .get(offset..end)
        .ok_or_else(|| CodecError::Message(format!("RomFS {what} exceeds the level 3 region")))
    };
    let expected_dir_hash_offset = L3_HEADER_SIZE;
    let expected_dir_meta_offset =
      dir_hash_offset
        .checked_add(dir_hash_length)
        .ok_or_else(|| {
          CodecError::Message("RomFS directory hash table range overflows".to_owned())
        })?;
    let expected_file_hash_offset = dir_meta_offset
      .checked_add(dir_meta_length)
      .ok_or_else(|| CodecError::Message("RomFS directory metadata range overflows".to_owned()))?;
    let expected_file_meta_offset = file_hash_offset
      .checked_add(file_hash_length)
      .ok_or_else(|| CodecError::Message("RomFS file hash table range overflows".to_owned()))?;
    let file_meta_end = file_meta_offset
      .checked_add(file_meta_length)
      .ok_or_else(|| CodecError::Message("RomFS file metadata range overflows".to_owned()))?;
    if dir_hash_offset != expected_dir_hash_offset
      || dir_meta_offset != expected_dir_meta_offset
      || file_hash_offset != expected_file_hash_offset
      || file_meta_offset != expected_file_meta_offset
      || data_offset < file_meta_end
      || !data_offset.is_multiple_of(DATA_ALIGN as usize)
    {
      return Err(CodecError::Message(
        "RomFS level 3 tables are not sequential or its file data offset is invalid".to_owned(),
      ));
    }
    let dir_table = section(dir_meta_offset, dir_meta_length, "directory metadata table")?;
    let file_table = section(file_meta_offset, file_meta_length, "file metadata table")?;
    let _ = section(dir_hash_offset, dir_hash_length, "directory hash table")?;
    let _ = section(file_hash_offset, file_hash_length, "file hash table")?;

    let dirs = parse_dir_table(dir_table)?;
    let files = parse_file_table(file_table)?;
    if dirs.is_empty() {
      return Err(CodecError::Message(
        "RomFS image has no root directory".to_owned(),
      ));
    }

    // Entry offsets within their table, in physical order.
    let offsets = |entries: &[RawEntry], fixed: usize| -> Result<Vec<u32>, CodecError> {
      let mut result = Vec::with_capacity(entries.len());
      let mut offset = 0u32;
      for entry in entries {
        result.push(offset);
        let name_size = u64::try_from(entry.name.len())
          .map_err(|_| CodecError::Message("RomFS entry name is impossibly large".to_owned()))?;
        let entry_size = (fixed as u64)
          .checked_add(align4(name_size))
          .and_then(|size| u32::try_from(size).ok())
          .ok_or_else(|| CodecError::Message("RomFS metadata table overflows".to_owned()))?;
        offset = offset
          .checked_add(entry_size)
          .ok_or_else(|| CodecError::Message("RomFS metadata table overflows".to_owned()))?;
      }
      Ok(result)
    };
    let dir_offsets = offsets(&dirs, DIR_ENTRY_FIXED)?;
    let _file_offsets = offsets(&files, FILE_ENTRY_FIXED)?;

    // The first directory is the root; parents always precede their children.
    if dirs[0].parent != 0 || !dirs[0].name.is_empty() {
      return Err(CodecError::Message(
        "RomFS directory table does not start with the root directory".to_owned(),
      ));
    }
    let mut dir_paths: Vec<String> = Vec::with_capacity(dirs.len());
    let mut directories = Vec::with_capacity(dirs.len());
    for (index, entry) in dirs.iter().enumerate() {
      let (name, name_raw) = decode_name(&entry.name, "directory")?;
      if (index > 0
        && (name.is_empty()
          || name == "."
          || name == ".."
          || name.contains('/')
          || name.contains('\\')))
        || (index == 0 && !name.is_empty())
      {
        return Err(CodecError::Message(format!(
          "RomFS directory name {name:?} cannot be used as an artifact path component"
        )));
      }
      let parent_index = if index == 0 {
        0
      } else {
        dir_offsets
          .iter()
          .position(|offset| *offset == entry.parent)
          .ok_or_else(|| {
            CodecError::Message(format!(
              "RomFS directory {index} points at unknown parent offset {:#x}",
              entry.parent
            ))
          })?
      };
      if index > 0 && parent_index >= index {
        return Err(CodecError::Message(format!(
          "RomFS directory {index} points at parent directory {parent_index}, which is not earlier in the directory table"
        )));
      }
      let parent_path = if index == 0 {
        String::new()
      } else {
        dir_paths[parent_index].clone()
      };
      let path = if parent_path.is_empty() {
        name.clone()
      } else {
        format!("{parent_path}/{name}")
      };
      directories.push(DirMetadata {
        parent: parent_path,
        name,
        name_raw,
      });
      dir_paths.push(path);
    }
    let mut unique_dir_paths = std::collections::HashSet::with_capacity(dir_paths.len());
    if dir_paths
      .iter()
      .any(|path| !unique_dir_paths.insert(path.as_str()))
    {
      return Err(CodecError::Message(
        "RomFS directory table contains duplicate paths".to_owned(),
      ));
    }

    let mut file_names: Vec<(String, String, Option<String>, u64, u64)> = Vec::new();
    for entry in &files {
      let (name, name_raw) = decode_name(&entry.name, "file")?;
      if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\')
      {
        return Err(CodecError::Message(format!(
          "RomFS file name {name:?} cannot be used as an artifact path component"
        )));
      }
      let parent_index = dir_offsets
        .iter()
        .position(|offset| *offset == entry.parent)
        .ok_or_else(|| {
          CodecError::Message(format!(
            "RomFS file {name:?} points at unknown parent offset {:#x}",
            entry.parent
          ))
        })?;
      file_names.push((
        dir_paths[parent_index].clone(),
        name,
        name_raw,
        entry.data_offset,
        entry.data_size,
      ));
    }

    // Validate sibling/hash links only lightly: the tree above already pins
    // the structure the rebuild needs. Artifact paths must still be unique.
    let mut unique_file_paths = std::collections::HashSet::with_capacity(file_names.len());
    if file_names.iter().any(|(parent, name, _, _, _)| {
      let path = if parent.is_empty() {
        name.clone()
      } else {
        format!("{parent}/{name}")
      };
      !unique_file_paths.insert(path)
    }) {
      return Err(CodecError::Message(
        "RomFS file table contains duplicate artifact paths".to_owned(),
      ));
    }
    let artifact_paths: Vec<String> = file_names
      .iter()
      .map(|(parent, name, _, _, _)| {
        if parent.is_empty() {
          name.clone()
        } else {
          format!("{parent}/{name}")
        }
      })
      .collect();
    validate_file_paths(artifact_paths.iter().map(String::as_str))?;

    let mut artifacts = Vec::with_capacity(files.len() + 1);
    let mut file_metadata = Vec::with_capacity(files.len());
    for (parent, name, name_raw, data_offset_value, data_size) in file_names {
      let relative = usize::try_from(data_offset_value).map_err(|_| {
        CodecError::Message(format!(
          "RomFS file {name:?} data offset is impossibly large"
        ))
      })?;
      let size = usize::try_from(data_size).map_err(|_| {
        CodecError::Message(format!("RomFS file {name:?} data size is impossibly large"))
      })?;
      let start = data_offset
        .checked_add(relative)
        .ok_or_else(|| CodecError::Message(format!("RomFS file {name:?} data offset overflows")))?;
      let end = start
        .checked_add(size)
        .filter(|&end| end <= level3.len())
        .ok_or_else(|| {
          CodecError::Message(format!(
            "RomFS file {name:?} data range exceeds the level 3 region"
          ))
        })?;
      let bytes = level3.get(start..end).ok_or_else(|| {
        CodecError::Message(format!(
          "RomFS file {name:?} data exceeds the level 3 region"
        ))
      })?;
      let path = if parent.is_empty() {
        name.clone()
      } else {
        format!("{parent}/{name}")
      };
      artifacts.push(DecodedArtifact {
        name: format!("{FILES_PREFIX}{path}"),
        data: bytes.to_vec(),
      });
      file_metadata.push(FileMetadata {
        parent,
        name,
        name_raw,
        data_offset: data_offset_value,
        size: data_size,
      });
    }

    if !dir_hash_length.is_multiple_of(4) || !file_hash_length.is_multiple_of(4) {
      return Err(CodecError::Message(
        "RomFS hash tables have a non-u32 length".to_owned(),
      ));
    }

    let metadata = RomfsMetadata {
      magic_id: header.magic_id,
      master_hash_size: header.master_hash_size,
      optional_info_size: header.optional_info_size,
      reserved_tail: hex::encode(header.reserved_tail),
      levels: header.levels,
      dir_hash_buckets: Some((dir_hash_length / 4) as u32),
      file_hash_buckets: Some((file_hash_length / 4) as u32),
      directories,
      files: file_metadata,
    };

    artifacts.insert(
      0,
      DecodedArtifact {
        name: METADATA_NAME.to_owned(),
        data: metadata.to_json()?,
      },
    );
    Ok(artifacts)
  }

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding Nintendo 3DS RomFS image");
    let _args = N3dsRomFsCodecArgs::from_args(args)?;

    let (metadata_artifact, file_artifacts) = artifacts.split_first().ok_or_else(|| {
      CodecError::Message(format!("Expected a {METADATA_NAME} artifact, found none"))
    })?;
    if metadata_artifact.name != METADATA_NAME {
      return Err(CodecError::Message(format!(
        "Expected the first artifact to be {METADATA_NAME}, found {}",
        metadata_artifact.name
      )));
    }
    let metadata = RomfsMetadata::from_json(&metadata_artifact.data)?;

    if metadata.directories.is_empty()
      || !metadata.directories[0].name.is_empty()
      || !metadata.directories[0].parent.is_empty()
    {
      return Err(CodecError::Message(
        "RomFS metadata must start with the root directory (empty name and parent)".to_owned(),
      ));
    }
    for directory in &metadata.directories[1..] {
      if directory.name.is_empty()
        || directory.name.contains('/')
        || directory.name.contains('\\')
        || directory.name == "."
        || directory.name == ".."
      {
        return Err(CodecError::Message(format!(
          "RomFS directory name {:?} cannot be used in a path",
          directory.name
        )));
      }
    }
    for file in &metadata.files {
      if file.name.is_empty()
        || file.name.contains('/')
        || file.name.contains('\\')
        || file.name == "."
        || file.name == ".."
      {
        return Err(CodecError::Message(format!(
          "RomFS file name {:?} cannot be used in a path",
          file.name
        )));
      }
    }

    // Validate raw-name placeholders too: the readable path name must stay
    // consistent with the bytes that will be written to the RomFS table.
    for directory in &metadata.directories {
      let raw = encode_name(&directory.name, &directory.name_raw)?;
      let (decoded, expected_raw) = decode_name(&raw, "directory")?;
      if decoded != directory.name || expected_raw != directory.name_raw {
        return Err(CodecError::Message(format!(
          "RomFS metadata directory name {:?} does not match its raw name",
          directory.name
        )));
      }
    }
    for file in &metadata.files {
      let raw = encode_name(&file.name, &file.name_raw)?;
      let (decoded, expected_raw) = decode_name(&raw, "file")?;
      if decoded != file.name || expected_raw != file.name_raw {
        return Err(CodecError::Message(format!(
          "RomFS metadata file name {:?} does not match its raw name",
          file.name
        )));
      }
    }

    // Match the artifacts to the declared files.
    let mut contents: Vec<&[u8]> = Vec::with_capacity(metadata.files.len());
    for file in &metadata.files {
      let path = if file.parent.is_empty() {
        file.name.clone()
      } else {
        format!("{}/{}", file.parent, file.name)
      };
      let expected = format!("{FILES_PREFIX}{path}");
      let matching: Vec<&[u8]> = file_artifacts
        .iter()
        .filter(|artifact| artifact.name == expected)
        .map(|artifact| artifact.data.as_slice())
        .collect();
      match matching.as_slice() {
        [] => {
          return Err(CodecError::Message(format!(
            "Expected a {expected} artifact, found none"
          )));
        }
        [bytes] => contents.push(bytes),
        _ => return Err(CodecError::DuplicateArtifact(expected)),
      }
    }
    for artifact in file_artifacts {
      let covered = metadata.files.iter().any(|file| {
        let path = if file.parent.is_empty() {
          file.name.clone()
        } else {
          format!("{}/{}", file.parent, file.name)
        };
        artifact.name == format!("{FILES_PREFIX}{path}")
      });
      if !covered {
        return Err(CodecError::Message(format!(
          "Unexpected artifact: {}",
          artifact.name
        )));
      }
    }

    // Names to UTF-16LE bytes.
    let dir_names: Vec<Vec<u8>> = metadata
      .directories
      .iter()
      .map(|directory| encode_name(&directory.name, &directory.name_raw))
      .collect::<Result<_, _>>()?;
    let file_names_raw: Vec<Vec<u8>> = metadata
      .files
      .iter()
      .map(|file| encode_name(&file.name, &file.name_raw))
      .collect::<Result<_, _>>()?;

    // Directory paths and parent lookups.
    let mut dir_paths: Vec<String> = Vec::with_capacity(metadata.directories.len());
    for directory in &metadata.directories {
      let path = if directory.parent.is_empty() && directory.name.is_empty() {
        String::new()
      } else if directory.parent.is_empty() {
        directory.name.clone()
      } else {
        format!("{}/{}", directory.parent, directory.name)
      };
      if dir_paths.contains(&path) {
        return Err(CodecError::Message(format!(
          "RomFS metadata describes {path:?} twice"
        )));
      }
      dir_paths.push(path);
    }
    let mut unique_dir_paths = std::collections::HashSet::with_capacity(dir_paths.len());
    if dir_paths
      .iter()
      .any(|path| !unique_dir_paths.insert(path.as_str()))
    {
      return Err(CodecError::Message(
        "RomFS metadata describes duplicate directory paths".to_owned(),
      ));
    }
    let dir_index = |path: &str| dir_paths.iter().position(|p| p == path);
    let artifact_paths: Vec<String> = metadata
      .files
      .iter()
      .map(|file| {
        if file.parent.is_empty() {
          file.name.clone()
        } else {
          format!("{}/{}", file.parent, file.name)
        }
      })
      .collect();
    validate_file_paths(artifact_paths.iter().map(String::as_str))?;

    // Entry offsets in physical order.
    let mut dir_offsets = Vec::with_capacity(metadata.directories.len());
    let mut offset = 0u32;
    for name in &dir_names {
      dir_offsets.push(offset);
      let entry_size = u32::try_from(dir_entry_size(name.len()))
        .map_err(|_| CodecError::Message("RomFS directory entry is impossibly large".to_owned()))?;
      offset = offset
        .checked_add(entry_size)
        .ok_or_else(|| CodecError::Message("RomFS directory table overflows".to_owned()))?;
    }
    let dir_table_len = offset as u64;
    let mut file_offsets = Vec::with_capacity(metadata.files.len());
    let mut offset = 0u32;
    for name in &file_names_raw {
      file_offsets.push(offset);
      let entry_size = u32::try_from(file_entry_size(name.len()))
        .map_err(|_| CodecError::Message("RomFS file entry is impossibly large".to_owned()))?;
      offset = offset
        .checked_add(entry_size)
        .ok_or_else(|| CodecError::Message("RomFS file table overflows".to_owned()))?;
    }
    let file_table_len = offset as u64;

    // File data offsets: reuse the recorded layout while it still fits.
    let mut data_offsets: Vec<u64> = Vec::with_capacity(metadata.files.len());
    let mut end = 0u64;
    let mut reusable = metadata
      .files
      .iter()
      .all(|file| file.data_offset % DATA_ALIGN == 0 || file.size == 0);
    if reusable {
      for (file, bytes) in metadata.files.iter().zip(&contents) {
        let recorded = if bytes.is_empty() {
          0
        } else {
          file.data_offset
        };
        if !bytes.is_empty() && recorded < end {
          reusable = false;
          break;
        }
        data_offsets.push(recorded);
        if !bytes.is_empty() {
          let Some(next_end) = u64::try_from(bytes.len())
            .ok()
            .and_then(|length| recorded.checked_add(length))
          else {
            reusable = false;
            break;
          };
          end = next_end;
        }
      }
    }
    if !reusable {
      debug!("RomFS data layout recomputed: the original offsets no longer fit");
      data_offsets.clear();
      end = 0;
      for bytes in &contents {
        if bytes.is_empty() {
          data_offsets.push(0);
          continue;
        }
        let aligned = align_up(end, DATA_ALIGN)
          .ok_or_else(|| CodecError::Message("RomFS data area overflows".to_owned()))?;
        data_offsets.push(aligned);
        let length = u64::try_from(bytes.len())
          .map_err(|_| CodecError::Message("RomFS file is impossibly large".to_owned()))?;
        end = aligned
          .checked_add(length)
          .ok_or_else(|| CodecError::Message("RomFS data area overflows".to_owned()))?;
      }
    }
    let data_len = end;

    // Hash tables, makerom rules: insertion in table order, new entries head
    // their bucket chain.
    let directory_count = u32::try_from(metadata.directories.len()).map_err(|_| {
      CodecError::Message("RomFS directory count does not fit the format".to_owned())
    })?;
    let file_count = u32::try_from(metadata.files.len())
      .map_err(|_| CodecError::Message("RomFS file count does not fit the format".to_owned()))?;
    let dir_buckets = metadata
      .dir_hash_buckets
      .unwrap_or_else(|| hash_table_count(directory_count)) as usize;
    let file_buckets = metadata
      .file_hash_buckets
      .unwrap_or_else(|| hash_table_count(file_count)) as usize;
    if dir_buckets == 0 || file_buckets == 0 {
      return Err(CodecError::Message(
        "RomFS metadata hash bucket counts must be positive".to_owned(),
      ));
    }
    let dir_hash_size = dir_buckets
      .checked_mul(4)
      .ok_or_else(|| CodecError::Message("RomFS directory hash table size overflows".to_owned()))?;
    let file_hash_size = file_buckets
      .checked_mul(4)
      .ok_or_else(|| CodecError::Message("RomFS file hash table size overflows".to_owned()))?;
    let dir_hash_size = u32::try_from(dir_hash_size).map_err(|_| {
      CodecError::Message("RomFS directory hash table does not fit the format".to_owned())
    })?;
    let file_hash_size = u32::try_from(file_hash_size).map_err(|_| {
      CodecError::Message("RomFS file hash table does not fit the format".to_owned())
    })?;
    let dir_table_capacity = usize::try_from(dir_table_len)
      .map_err(|_| CodecError::Message("RomFS directory table is impossibly large".to_owned()))?;
    let file_table_capacity = usize::try_from(file_table_len)
      .map_err(|_| CodecError::Message("RomFS file table is impossibly large".to_owned()))?;
    let mut dir_bytes = Vec::with_capacity(dir_table_capacity);
    let mut file_bytes = Vec::with_capacity(file_table_capacity);
    let mut dir_hash = vec![UNUSED_ENTRY; dir_buckets];
    let mut file_hash = vec![UNUSED_ENTRY; file_buckets];

    for (index, directory) in metadata.directories.iter().enumerate() {
      let parent_offset = if index == 0 {
        0
      } else {
        let parent_index = dir_index(&directory.parent).ok_or_else(|| {
          CodecError::Message(format!(
            "RomFS directory {:?} has unknown parent {:?}",
            directory.name, directory.parent
          ))
        })?;
        if parent_index >= index {
          return Err(CodecError::Message(format!(
            "RomFS directory {:?} is listed before its parent",
            directory.name
          )));
        }
        dir_offsets[parent_index]
      };

      let bucket = (calc_path_hash(parent_offset, &dir_names[index]) as usize) % dir_buckets;
      let hash_next = dir_hash[bucket];
      dir_hash[bucket] = dir_offsets[index];

      dir_bytes.extend_from_slice(&parent_offset.to_le_bytes());
      dir_bytes.extend_from_slice(&UNUSED_ENTRY.to_le_bytes()); // sibling, patched below
      dir_bytes.extend_from_slice(&UNUSED_ENTRY.to_le_bytes()); // child, patched below
      dir_bytes.extend_from_slice(&UNUSED_ENTRY.to_le_bytes()); // first file, patched below
      dir_bytes.extend_from_slice(&hash_next.to_le_bytes());
      dir_bytes.extend_from_slice(&(dir_names[index].len() as u32).to_le_bytes());
      dir_bytes.extend_from_slice(&dir_names[index]);
      dir_bytes.extend(std::iter::repeat_n(
        0,
        (align4(dir_names[index].len() as u64) as usize) - dir_names[index].len(),
      ));
    }

    for (index, file) in metadata.files.iter().enumerate() {
      let parent_index = dir_index(&file.parent).ok_or_else(|| {
        CodecError::Message(format!(
          "RomFS file {:?} has unknown parent {:?}",
          file.name, file.parent
        ))
      })?;
      let parent_offset = dir_offsets[parent_index];
      let bucket = (calc_path_hash(parent_offset, &file_names_raw[index]) as usize) % file_buckets;
      let hash_next = file_hash[bucket];
      file_hash[bucket] = file_offsets[index];

      file_bytes.extend_from_slice(&parent_offset.to_le_bytes());
      file_bytes.extend_from_slice(&UNUSED_ENTRY.to_le_bytes()); // sibling, patched below
      file_bytes.extend_from_slice(&data_offsets[index].to_le_bytes());
      file_bytes.extend_from_slice(&(contents[index].len() as u64).to_le_bytes());
      file_bytes.extend_from_slice(&hash_next.to_le_bytes());
      file_bytes.extend_from_slice(&(file_names_raw[index].len() as u32).to_le_bytes());
      file_bytes.extend_from_slice(&file_names_raw[index]);
      file_bytes.extend(std::iter::repeat_n(
        0,
        (align4(file_names_raw[index].len() as u64) as usize) - file_names_raw[index].len(),
      ));
    }

    // Sibling/child links: children of a directory in metadata order.
    let patch_u32 = |table: &mut [u8], entry_offset: u32, field_offset: usize, value: u32| {
      let at = entry_offset as usize + field_offset;
      table[at..at + 4].copy_from_slice(&value.to_le_bytes());
    };
    for (index, _directory) in metadata.directories.iter().enumerate() {
      let path = &dir_paths[index];
      let child_dirs: Vec<usize> = metadata
        .directories
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, d)| &d.parent == path)
        .map(|(i, _)| i)
        .collect();
      if let Some(first) = child_dirs.first() {
        patch_u32(&mut dir_bytes, dir_offsets[index], 0x8, dir_offsets[*first]);
      }
      for pair in child_dirs.windows(2) {
        patch_u32(
          &mut dir_bytes,
          dir_offsets[pair[0]],
          0x4,
          dir_offsets[pair[1]],
        );
      }

      let child_files: Vec<usize> = metadata
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| &f.parent == path)
        .map(|(i, _)| i)
        .collect();
      if let Some(first) = child_files.first() {
        patch_u32(
          &mut dir_bytes,
          dir_offsets[index],
          0xC,
          file_offsets[*first],
        );
      }
      for pair in child_files.windows(2) {
        patch_u32(
          &mut file_bytes,
          file_offsets[pair[0]],
          0x4,
          file_offsets[pair[1]],
        );
      }
    }
    // Assemble level 3.
    let mut level3 = Vec::new();
    level3.extend_from_slice(&[0; L3_HEADER_SIZE]); // header, patched below
    let dir_hash_offset = u32::try_from(level3.len()).map_err(|_| {
      CodecError::Message("RomFS level 3 offset does not fit the header".to_owned())
    })?;
    for bucket in &dir_hash {
      level3.extend_from_slice(&bucket.to_le_bytes());
    }
    let dir_meta_offset = u32::try_from(level3.len()).map_err(|_| {
      CodecError::Message("RomFS level 3 offset does not fit the header".to_owned())
    })?;
    level3.extend_from_slice(&dir_bytes);
    let file_hash_offset = u32::try_from(level3.len()).map_err(|_| {
      CodecError::Message("RomFS level 3 offset does not fit the header".to_owned())
    })?;
    for bucket in &file_hash {
      level3.extend_from_slice(&bucket.to_le_bytes());
    }
    let file_meta_offset = u32::try_from(level3.len()).map_err(|_| {
      CodecError::Message("RomFS level 3 offset does not fit the header".to_owned())
    })?;
    level3.extend_from_slice(&file_bytes);
    let data_offset = align_up(level3.len() as u64, DATA_ALIGN)
      .ok_or_else(|| CodecError::Message("RomFS header overflow".to_owned()))?;
    let data_offset_padding = usize::try_from(data_offset - level3.len() as u64)
      .map_err(|_| CodecError::Message("RomFS data alignment is impossibly large".to_owned()))?;
    level3.extend(std::iter::repeat_n(0, data_offset_padding));
    for (index, bytes) in contents.iter().enumerate() {
      if bytes.is_empty() {
        continue;
      }
      let at = data_offset
        .checked_add(data_offsets[index])
        .ok_or_else(|| CodecError::Message("RomFS file data offset overflows".to_owned()))?;
      let gap = at
        .checked_sub(level3.len() as u64)
        .ok_or_else(|| CodecError::Message("RomFS file data overlaps its metadata".to_owned()))?;
      let gap = usize::try_from(gap)
        .map_err(|_| CodecError::Message("RomFS file gap is impossibly large".to_owned()))?;
      level3.extend(std::iter::repeat_n(0, gap));
      level3.extend_from_slice(bytes);
    }
    let level3_size = data_offset
      .checked_add(data_len)
      .ok_or_else(|| CodecError::Message("RomFS level 3 size overflows".to_owned()))?
      .max(level3.len() as u64);

    let dir_table_len = u32::try_from(dir_table_len).map_err(|_| {
      CodecError::Message("RomFS directory table does not fit the header".to_owned())
    })?;
    let file_table_len = u32::try_from(file_table_len)
      .map_err(|_| CodecError::Message("RomFS file table does not fit the header".to_owned()))?;
    let data_offset = u32::try_from(data_offset)
      .map_err(|_| CodecError::Message("RomFS data offset does not fit the header".to_owned()))?;
    let mut l3_header = Vec::with_capacity(L3_HEADER_SIZE);
    l3_header.extend_from_slice(&(L3_HEADER_SIZE as u32).to_le_bytes());
    l3_header.extend_from_slice(&dir_hash_offset.to_le_bytes());
    l3_header.extend_from_slice(&dir_hash_size.to_le_bytes());
    l3_header.extend_from_slice(&dir_meta_offset.to_le_bytes());
    l3_header.extend_from_slice(&dir_table_len.to_le_bytes());
    l3_header.extend_from_slice(&file_hash_offset.to_le_bytes());
    l3_header.extend_from_slice(&file_hash_size.to_le_bytes());
    l3_header.extend_from_slice(&file_meta_offset.to_le_bytes());
    l3_header.extend_from_slice(&file_table_len.to_le_bytes());
    l3_header.extend_from_slice(&data_offset.to_le_bytes());
    level3[..L3_HEADER_SIZE].copy_from_slice(&l3_header);
    let level3_size = usize::try_from(level3_size)
      .map_err(|_| CodecError::Message("RomFS level 3 is impossibly large".to_owned()))?;
    level3.resize(level3_size, 0);

    // IVFC hash tree: level 2 hashes level 3, level 1 hashes level 2, the
    // master hash hashes level 1, each in that level's block size.
    let block = |index: usize| -> Result<u64, CodecError> {
      let log2 = metadata.levels[index].block_size_log2;
      if log2 >= 32 {
        return Err(CodecError::Message(format!(
          "RomFS level {} block size 2^{log2} is implausible",
          index + 1
        )));
      }
      Ok(1u64 << log2)
    };
    let level2 = hash_level(&level3, block(2)?)?;
    let level1 = hash_level(&level2, block(1)?)?;
    let master = hash_level(&level1, block(0)?)?;
    let master_size = u32::try_from(master.len())
      .map_err(|_| CodecError::Message("RomFS master hash does not fit the header".to_owned()))?;

    // Physical layout: level 0 (header + master hash), then Levels 3, 1, 2.
    // Logical offsets are a separate address space linking the hash layers.
    let level0_size = LEVEL0_HEADER_SIZE
      .checked_add(u64::from(master_size))
      .ok_or_else(|| CodecError::Message("RomFS level 0 size overflows".to_owned()))?;
    let level3_offset = align_up(level0_size, block(2)?)
      .ok_or_else(|| CodecError::Message("RomFS layout overflows".to_owned()))?;
    let level3_end = level3_offset
      .checked_add(level3.len() as u64)
      .ok_or_else(|| CodecError::Message("RomFS layout overflows".to_owned()))?;
    let level1_offset = align_up(level3_end, block(2)?)
      .ok_or_else(|| CodecError::Message("RomFS layout overflows".to_owned()))?;
    let level1_end = level1_offset
      .checked_add(level1.len() as u64)
      .ok_or_else(|| CodecError::Message("RomFS layout overflows".to_owned()))?;
    let level2_offset = align_up(level1_end, block(0)?)
      .ok_or_else(|| CodecError::Message("RomFS layout overflows".to_owned()))?;
    let level2_end = level2_offset
      .checked_add(level2.len() as u64)
      .ok_or_else(|| CodecError::Message("RomFS layout overflows".to_owned()))?;
    let total = align_up(level2_end, block(1)?)
      .ok_or_else(|| CodecError::Message("RomFS layout overflows".to_owned()))?;

    let logical_level1 = 0;
    let logical_level2 = align_up(level1.len() as u64, block(0)?)
      .ok_or_else(|| CodecError::Message("RomFS logical layout overflows".to_owned()))?;
    let logical_level3 = align_up(
      logical_level2
        .checked_add(level2.len() as u64)
        .ok_or_else(|| CodecError::Message("RomFS logical layout overflows".to_owned()))?,
      block(1)?,
    )
    .ok_or_else(|| CodecError::Message("RomFS logical layout overflows".to_owned()))?;

    let total = usize::try_from(total)
      .map_err(|_| CodecError::Message("RomFS image is impossibly large".to_owned()))?;
    let mut image = vec![0u8; total];
    image[..4].copy_from_slice(IVFC_MAGIC);
    image[0x4..0x8].copy_from_slice(&metadata.magic_id.to_le_bytes());
    image[MASTER_HASH_SIZE_OFFSET..MASTER_HASH_SIZE_OFFSET + 4]
      .copy_from_slice(&master_size.to_le_bytes());
    let physical_offsets = [
      level1_offset,
      level2_offset,
      level3_offset,
    ];
    let logical_offsets = [
      logical_level1,
      logical_level2,
      logical_level3,
    ];
    let level_datas: [&[u8]; 3] = [
      &level1,
      &level2,
      &level3,
    ];
    for (index, (((physical_offset, logical_offset), level_data), level_meta)) in physical_offsets
      .iter()
      .zip(logical_offsets.iter())
      .zip(level_datas.iter())
      .zip(metadata.levels.iter())
      .enumerate()
    {
      let base = LEVEL_HEADERS_OFFSET + index * LEVEL_HEADER_SIZE;
      image[base..base + 8].copy_from_slice(&logical_offset.to_le_bytes());
      let level_size = u64::try_from(level_data.len())
        .map_err(|_| CodecError::Message("RomFS level is impossibly large".to_owned()))?;
      image[base + 8..base + 0x10].copy_from_slice(&level_size.to_le_bytes());
      image[base + 0x10..base + 0x14].copy_from_slice(&level_meta.block_size_log2.to_le_bytes());
      image[base + 0x14..base + 0x18].copy_from_slice(&level_meta.reserved.to_le_bytes());
      let at = usize::try_from(*physical_offset)
        .map_err(|_| CodecError::Message("RomFS level offset is impossibly large".to_owned()))?;
      let end = at
        .checked_add(level_data.len())
        .ok_or_else(|| CodecError::Message("RomFS level range overflows the image".to_owned()))?;
      image
        .get_mut(at..end)
        .ok_or_else(|| CodecError::Message("RomFS level exceeds the output image".to_owned()))?
        .copy_from_slice(level_data);
    }
    let reserved_tail = hex_field(&metadata.reserved_tail, "reserved tail", RESERVED_TAIL_SIZE)?;
    image[RESERVED_TAIL_OFFSET..RESERVED_TAIL_OFFSET + RESERVED_TAIL_SIZE]
      .copy_from_slice(&reserved_tail);
    image[OPTIONAL_INFO_SIZE_OFFSET..OPTIONAL_INFO_SIZE_OFFSET + 4]
      .copy_from_slice(&metadata.optional_info_size.to_le_bytes());
    let master_start = usize::try_from(LEVEL0_HEADER_SIZE).map_err(|_| {
      CodecError::Message("RomFS master hash offset is impossibly large".to_owned())
    })?;
    let master_end = master_start
      .checked_add(master.len())
      .ok_or_else(|| CodecError::Message("RomFS master hash range overflows".to_owned()))?;
    image
      .get_mut(master_start..master_end)
      .ok_or_else(|| CodecError::Message("RomFS master hash exceeds the output image".to_owned()))?
      .copy_from_slice(&master);

    Ok(image)
  }
}
