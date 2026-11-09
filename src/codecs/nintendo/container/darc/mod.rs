//! Nintendo DARC (DS/3DS archive) codec.
//!
//! DARC is Nintendo's archive format for the DS and 3DS: a header, a flat
//! folder/file table in depth-first order, a UTF-16 name area and a data area.
//! The container can be big- or little-endian; the
//! byte-order marker selects which. Most 3DS archives are little-endian and
//! are frequently stored LZ10/LZ11-compressed, so this codec is the natural
//! target of the `darc` preset of those codecs.
//!
//! Format references:
//! - <https://www.3dbrew.org/wiki/DARC>
//! - <https://problemkaputt.de/gbatek.htm#3dsfilesarchivedarc>
//!
//! The rebuild preserves the decoded header fields, table order, name area and
//! file offsets while they still describe the artifacts, so an unmodified
//! rebuild is byte-identical. Files that grow can use available padding. If a
//! relayout is needed, repeated exact padding patterns in the source layout
//! provide conservative per-file alignment evidence; otherwise files use a
//! 4-byte fallback alignment.

use super::super::n3ds::common::align_up;
use super::{
  Endian, FILES_PREFIX, Gap, collect_gaps, decode_hex, is_zero_u64, join_trailing, split_trailing,
  validate_file_paths, write_gaps,
};
use crate::codecs::errors::CodecError;
use crate::codecs::{Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact, LudaredCodecArgs};
use log::{debug, trace};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;

#[cfg(test)]
mod tests;

const CODEC_ID: &str = "std/nintendo/container/darc";
const CODEC_NAME: &str = "Nintendo DARC archive";
const CODEC_DESC: &str = r#"Nintendo DARC archive

Extracts a DARC archive into:
- DARC.json: header fields, table order, UTF-16 name bytes and file layout,
  including any non-zero padding, so an unmodified rebuild stays byte-identical.
- One artifact per file, under files/ with its full path.

The header can be big- or little-endian; the byte-order marker is preserved.
Folders and the optional leading "." entry are described in DARC.json rather
than emitted as artifacts, because they carry no data.

Rebuilding preserves the decoded table order, name area and file offsets while
they still fit the artifacts. Files can grow into available padding. If an
edited table no longer fits or file ranges conflict, the data area is relaid
out using repeated padding patterns observed in the source archive; files
without sufficient evidence use 4-byte alignment. Names that are not valid
UTF-16 keep their raw bytes in
DARC.json.

No arguments.
"#;

const MAGIC: &[u8; 4] = b"darc";
const HEADER_SIZE: usize = 0x1C;
const ENTRY_SIZE: usize = 12;
const FOLDER_FLAG: u32 = 0x0100_0000;
const NAME_MASK: u32 = 0x00FF_FFFF;
const FALLBACK_DATA_ALIGN: u64 = 4;
const MIN_ALIGNMENT_EVIDENCE: usize = 3;
const MAX_INFERRED_ALIGNMENT: u64 = 1 << 31;
const METADATA_NAME: &str = "DARC.json";

pub(crate) struct Darc;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DarcCodecArgs {}

impl LudaredCodecArgs for DarcCodecArgs {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum EntryKind {
  File,
  Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryMetadata {
  #[serde(rename = "type")]
  kind: EntryKind,
  /// Full path of the containing folder ("" for the root's children).
  parent: String,
  /// UTF-8 view of the UTF-16 name.
  name: String,
  /// Raw UTF-16 bytes when they do not round-trip through UTF-8.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  name_raw: Option<String>,
  /// Offset within the name area, as decoded.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  name_offset: Option<u32>,
  /// Absolute offset within the archive (files only).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  data_offset: Option<u32>,
  /// Size in bytes as decoded (files only).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  size: Option<u32>,
  /// Alignment supported by repeated padding evidence in the decoded layout.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  data_alignment: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DarcMetadata {
  endian: Endian,
  header_size: u16,
  version: u32,
  table_offset: u32,
  data_offset: u32,
  /// Bytes between the fixed header and the table; empty when all zero.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  prefix: String,
  /// Raw name area; empty only when the archive has no names.
  #[serde(default)]
  name_area: String,
  /// Bytes between the name area and the data area; empty when all zero.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  table_padding: String,
  /// Length of the data area.
  data_size: u32,
  /// Non-zero bytes between file data.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  gaps: Vec<Gap>,
  /// Bytes after the archive's declared file size, when any is non-zero.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  trailing: String,
  /// Length of the zero padding after the declared file size.
  #[serde(default, skip_serializing_if = "is_zero_u64")]
  trailing_zeros: u64,
  /// Whether entry 1 is the transparent "." folder.
  dot_entry: bool,
  root_name_offset: u32,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  dot_name_offset: Option<u32>,
  /// Real entries in physical table order, excluding the root and "." entries.
  entries: Vec<EntryMetadata>,
}

impl DarcMetadata {
  fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the DARC metadata: {err}")))
  }

  fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the DARC metadata artifact: {err}"))
    })
  }
}

/// The fixed archive header that precedes the entry table.
#[derive(Debug)]
struct ArchiveHeader {
  endian: Endian,
  /// Size of this header, in bytes.
  header_size: u16,
  version: u32,
  file_size: u32,
  table_offset: u32,
  table_size: u32,
  data_offset: u32,
}

#[derive(Debug)]
struct RawEntry {
  name_offset: u32,
  is_folder: bool,
  /// Folder: parent index. File: absolute data offset.
  second: u32,
  /// Folder: exclusive end index. File: size.
  third: u32,
  name: String,
  name_raw: Option<String>,
}

struct ParsedDarc {
  header: ArchiveHeader,
  entries: Vec<RawEntry>,
  paths: Vec<String>,
  dot_entry: bool,
  name_area_start: usize,
  name_area_len: usize,
}

fn parse_header(data: &[u8]) -> Result<ArchiveHeader, CodecError> {
  let header = data.get(..HEADER_SIZE).ok_or_else(|| {
    CodecError::Message(format!(
      "DARC header is truncated: {} bytes (minimum: {HEADER_SIZE})",
      data.len()
    ))
  })?;
  if header[..4] != MAGIC[..] {
    return Err(CodecError::Message(
      "Not a DARC archive: the 'darc' magic is missing".to_owned(),
    ));
  }
  let endian = Endian::from_bom(header, 4)?;
  let header_size = endian.read_u16(header, 0x06)?;
  let version = endian.read_u32(header, 0x08)?;
  let file_size = endian.read_u32(header, 0x0C)?;
  let table_offset = endian.read_u32(header, 0x10)?;
  let table_size = endian.read_u32(header, 0x14)?;
  let data_offset = endian.read_u32(header, 0x18)?;

  if usize::from(header_size) < HEADER_SIZE {
    return Err(CodecError::Message(format!(
      "DARC header is {header_size} bytes, expected at least {HEADER_SIZE}"
    )));
  }
  if table_offset < u32::from(header_size) {
    return Err(CodecError::Message(format!(
      "DARC table offset {table_offset:#x} is before the end of its header ({header_size:#x})"
    )));
  }
  if (table_size as usize) < ENTRY_SIZE {
    return Err(CodecError::Message(format!(
      "DARC table is {table_size} bytes, too small for one entry"
    )));
  }
  let table_end = table_offset
    .checked_add(table_size)
    .ok_or_else(|| CodecError::Message("DARC table range overflows".to_owned()))?;
  if data_offset < table_end {
    return Err(CodecError::Message(format!(
      "DARC data offset {data_offset:#x} is before the end of its table ({table_end:#x})"
    )));
  }
  if file_size as usize > data.len() {
    return Err(CodecError::Message(format!(
      "DARC declares {} bytes but the input is only {} bytes",
      file_size,
      data.len()
    )));
  }
  if file_size < data_offset {
    return Err(CodecError::Message(format!(
      "DARC file size {file_size:#x} is before its data area ({data_offset:#x})"
    )));
  }

  Ok(ArchiveHeader {
    endian,
    header_size,
    version,
    file_size,
    table_offset,
    table_size,
    data_offset,
  })
}

/// Reads a NUL-terminated UTF-16 name from `name_area`.
fn read_name(name_area: &[u8], offset: usize, endian: Endian) -> Result<Vec<u8>, CodecError> {
  let tail = name_area.get(offset..).ok_or_else(|| {
    CodecError::Message(format!(
      "DARC name offset {offset:#x} exceeds the name area ({} bytes)",
      name_area.len()
    ))
  })?;
  let (chunks, remainder) = tail.as_chunks::<2>();
  for (index, pair) in chunks.iter().enumerate() {
    let unit = match endian {
      Endian::Little => u16::from_le_bytes(*pair),
      Endian::Big => u16::from_be_bytes(*pair),
    };
    if unit == 0 {
      return Ok(tail[..index * 2].to_vec());
    }
  }
  Err(CodecError::Message(if remainder.is_empty() {
    format!("DARC name at {offset:#x} is not NUL-terminated")
  } else {
    format!("DARC name at {offset:#x} ends in an incomplete UTF-16 unit")
  }))
}

fn decode_name(
  raw: &[u8],
  endian: Endian,
  what: &str,
) -> Result<(String, Option<String>), CodecError> {
  if !raw.len().is_multiple_of(2) {
    return Err(CodecError::Message(format!(
      "DARC {what} name has an odd length of {} bytes",
      raw.len()
    )));
  }
  let units = endian.utf16_units(raw);
  match String::from_utf16(&units) {
    Ok(name) if endian.utf16_bytes(name.encode_utf16()) == raw => Ok((name, None)),
    _ => Ok((
      format!("_darc_raw_{}", hex::encode(raw)),
      Some(hex::encode(raw)),
    )),
  }
}

fn encode_name(
  name: &str,
  name_raw: Option<&str>,
  endian: Endian,
  what: &str,
) -> Result<Vec<u8>, CodecError> {
  let raw = match name_raw {
    Some(raw) => decode_hex(raw, what)?,
    None => endian.utf16_bytes(name.encode_utf16()),
  };
  if raw.len() % 2 != 0 {
    return Err(CodecError::Message(format!(
      "DARC metadata {what} name {name:?} has an odd length of {} bytes",
      raw.len()
    )));
  }
  if endian.utf16_units(&raw).contains(&0) {
    return Err(CodecError::Message(format!(
      "DARC metadata {what} name {name:?} contains a NUL code unit"
    )));
  }
  let (decoded, decoded_raw) = decode_name(&raw, endian, what)?;
  if decoded != name || decoded_raw.as_deref() != name_raw {
    return Err(CodecError::Message(format!(
      "DARC metadata {what} name {name:?} does not match its raw name"
    )));
  }
  Ok(raw)
}

fn parse_entries(
  data: &[u8],
  header: &ArchiveHeader,
) -> Result<(Vec<RawEntry>, usize, usize), CodecError> {
  let table_offset = header.table_offset as usize;
  let first = header.endian.read_u32(data, table_offset)?;
  if first & FOLDER_FLAG == 0 {
    return Err(CodecError::Message(
      "DARC entry 0 is not the root folder".to_owned(),
    ));
  }
  let count = header.endian.read_u32(data, table_offset + 0x8)? as usize;
  if count == 0 {
    return Err(CodecError::Message(
      "DARC root folder declares no entries".to_owned(),
    ));
  }
  let entry_bytes = count
    .checked_mul(ENTRY_SIZE)
    .ok_or_else(|| CodecError::Message("DARC entry table size overflows".to_owned()))?;
  let table_size = header.table_size as usize;
  if entry_bytes > table_size {
    return Err(CodecError::Message(format!(
      "DARC root folder declares {count} entries, which do not fit the {table_size}-byte table"
    )));
  }
  let name_area_start = table_offset
    .checked_add(entry_bytes)
    .ok_or_else(|| CodecError::Message("DARC name area offset overflows".to_owned()))?;
  let name_area_len = table_size - entry_bytes;
  let name_area = data
    .get(name_area_start..name_area_start + name_area_len)
    .ok_or_else(|| CodecError::Message("DARC name area exceeds the input".to_owned()))?;

  let mut entries = Vec::with_capacity(count);
  for index in 0..count {
    let base = table_offset + index * ENTRY_SIZE;
    let name_field = header.endian.read_u32(data, base)?;
    let second = header.endian.read_u32(data, base + 0x4)?;
    let third = header.endian.read_u32(data, base + 0x8)?;
    let name_offset = name_field & NAME_MASK;
    if !name_offset.is_multiple_of(2) {
      return Err(CodecError::Message(format!(
        "DARC entry {index} name offset {name_offset:#x} is not aligned to a UTF-16 unit"
      )));
    }
    let raw = read_name(name_area, name_offset as usize, header.endian)?;
    let (name, name_raw) = decode_name(&raw, header.endian, "entry")?;
    entries.push(RawEntry {
      name_offset,
      is_folder: name_field & FOLDER_FLAG != 0,
      second,
      third,
      name,
      name_raw,
    });
  }
  Ok((entries, name_area_start, name_area_len))
}

fn join_path(parent: &str, name: &str) -> String {
  if parent.is_empty() {
    name.to_owned()
  } else {
    format!("{parent}/{name}")
  }
}

fn parent_path(path: &str) -> &str {
  path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn validate_component(name: &str) -> Result<(), CodecError> {
  if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
    return Err(CodecError::Message(format!(
      "DARC entry name {name:?} cannot be used as a path component"
    )));
  }
  Ok(())
}

fn is_descendant(parent: &str, folder: &str) -> bool {
  if folder.is_empty() {
    return true;
  }
  parent == folder
    || parent
      .strip_prefix(folder)
      .is_some_and(|rest| rest.starts_with('/'))
}

/// Resolves the physical table order into full paths and validates the tree.
fn resolve_tree(entries: &[RawEntry]) -> Result<(Vec<String>, bool), CodecError> {
  let count = entries.len();
  let root = &entries[0];
  if !root.is_folder || !root.name.is_empty() || root.second != 0 || root.third as usize != count {
    return Err(CodecError::Message(
      "DARC entry 0 is not a root folder covering the whole table".to_owned(),
    ));
  }

  let dot_entry = count > 1
    && entries[1].is_folder
    && entries[1].name == "."
    && entries[1].second == 0
    && entries[1].third as usize == count;
  let first = if dot_entry { 2 } else { 1 };
  // A "." entry is the real root of the tree: the entries it contains point at
  // it rather than at the synthetic root at index 0.
  let root_parent = usize::from(dot_entry);

  let mut paths = vec![String::new(); count];
  let mut seen = HashSet::with_capacity(count);
  scan_folder(
    entries,
    &mut paths,
    &mut seen,
    first,
    count,
    root_parent,
    "",
  )?;
  Ok((paths, dot_entry))
}

fn scan_folder(
  entries: &[RawEntry],
  paths: &mut [String],
  seen: &mut HashSet<String>,
  start: usize,
  end: usize,
  parent_index: usize,
  parent_path: &str,
) -> Result<(), CodecError> {
  let mut index = start;
  while index < end {
    let entry = &entries[index];
    let path = join_path(parent_path, &entry.name);
    if !seen.insert(path.clone()) {
      return Err(CodecError::Message(format!(
        "DARC table contains duplicate path {path:?}"
      )));
    }
    paths[index].clone_from(&path);

    if entry.is_folder {
      if entry.second as usize != parent_index {
        return Err(CodecError::Message(format!(
          "DARC folder {path:?} points at parent index {}, expected {parent_index}",
          entry.second
        )));
      }
      let folder_end = entry.third as usize;
      if folder_end < index + 1 || folder_end > end {
        return Err(CodecError::Message(format!(
          "DARC folder {path:?} has invalid end index {folder_end}"
        )));
      }
      scan_folder(entries, paths, seen, index + 1, folder_end, index, &path)?;
      index = folder_end;
    } else {
      index += 1;
    }
  }
  Ok(())
}

fn parse(data: &[u8]) -> Result<ParsedDarc, CodecError> {
  let header = parse_header(data)?;
  let (entries, name_area_start, name_area_len) = parse_entries(data, &header)?;
  let (paths, dot_entry) = resolve_tree(&entries)?;
  Ok(ParsedDarc {
    header,
    entries,
    paths,
    dot_entry,
    name_area_start,
    name_area_len,
  })
}

/// Validates file ranges and returns them sorted by offset.
fn file_ranges(parsed: &ParsedDarc) -> Result<Vec<(usize, usize)>, CodecError> {
  let data_offset = parsed.header.data_offset as usize;
  let file_size = parsed.header.file_size as usize;
  let mut ranges = Vec::new();
  for (index, entry) in parsed.entries.iter().enumerate() {
    if entry.is_folder {
      continue;
    }
    let start = entry.second as usize;
    let size = entry.third as usize;
    let end = start
      .checked_add(size)
      .ok_or_else(|| CodecError::Message("DARC file range overflows".to_owned()))?;
    if start < data_offset || end > file_size {
      return Err(CodecError::Message(format!(
        "DARC file {} range {start:#x}..{end:#x} is outside the data area {data_offset:#x}..{file_size:#x}",
        parsed.paths[index]
      )));
    }
    if size > 0 {
      ranges.push((start, end));
    }
  }
  ranges.sort_unstable();
  for pair in ranges.windows(2) {
    if pair[1].0 < pair[0].1 {
      return Err(CodecError::Message(format!(
        "DARC file data ranges {:#x}..{:#x} and {:#x}..{:#x} overlap",
        pair[0].0, pair[0].1, pair[1].0, pair[1].1
      )));
    }
  }
  Ok(ranges)
}

/// Infers per-file alignment only when the source repeatedly uses the exact
/// padding needed to reach the same power-of-two boundary. A start offset
/// being divisible by an alignment is not evidence by itself.
fn infer_data_alignments(entries: &[EntryMetadata], data_offset: u64) -> Vec<Option<u32>> {
  let mut ordered: Vec<(usize, u64, u64)> = entries
    .iter()
    .enumerate()
    .filter_map(|(index, entry)| {
      if entry.kind != EntryKind::File {
        return None;
      }
      Some((index, u64::from(entry.data_offset?), u64::from(entry.size?)))
    })
    .filter(|(_, _, size)| *size > 0)
    .collect();
  ordered.sort_unstable_by_key(|(_, start, _)| *start);

  let mut evidence = HashMap::<u64, usize>::new();
  let mut candidates = Vec::with_capacity(ordered.len());
  let mut previous_end = data_offset;
  for &(index, start, size) in &ordered {
    let candidate = if start > previous_end {
      inferred_padding_alignment(previous_end, start)
    } else {
      None
    };
    if let Some(alignment) = candidate {
      *evidence.entry(alignment).or_default() += 1;
    }
    candidates.push((index, candidate));
    previous_end = start.saturating_add(size);
  }

  let mut alignments = vec![None; entries.len()];
  for (index, candidate) in candidates {
    if let Some(alignment) = candidate.filter(|alignment| {
      *alignment == FALLBACK_DATA_ALIGN
        || evidence.get(alignment).copied().unwrap_or_default() >= MIN_ALIGNMENT_EVIDENCE
    }) && alignment > FALLBACK_DATA_ALIGN
    {
      alignments[index] = u32::try_from(alignment).ok();
    }
  }
  alignments
}

fn inferred_padding_alignment(previous_end: u64, start: u64) -> Option<u64> {
  let mut alignment = FALLBACK_DATA_ALIGN;
  while alignment <= MAX_INFERRED_ALIGNMENT {
    if align_up(previous_end, alignment) == Some(start) {
      return Some(alignment);
    }
    alignment *= 2;
  }
  None
}

/// Moves preserved non-zero gap bytes with their nearest original file
/// boundary during a full relayout. If the gap cannot fit without overlap,
/// fail rather than discard or overwrite it.
fn relocate_gaps(
  metadata: &DarcMetadata,
  entries: &[&EntryMetadata],
  contents: &[&[u8]],
  offsets: &[u32],
  data_offset: u64,
) -> Result<Vec<(u64, Vec<u8>)>, CodecError> {
  let mut old_files: Vec<(u64, u64, usize)> = entries
    .iter()
    .enumerate()
    .filter_map(|(index, entry)| {
      let start = u64::from(entry.data_offset?);
      let size = u64::from(entry.size?);
      (size > 0).then_some((start, start + size, index))
    })
    .collect();
  old_files.sort_unstable_by_key(|(start, _, _)| *start);

  let new_files: Vec<(u64, u64)> = offsets
    .iter()
    .zip(contents)
    .filter(|(_, bytes)| !bytes.is_empty())
    .map(|(&start, bytes)| (u64::from(start), u64::from(start) + bytes.len() as u64))
    .collect();
  let mut relocated = Vec::with_capacity(metadata.gaps.len());
  for gap in &metadata.gaps {
    let bytes = decode_hex(&gap.data, "gap")?;
    let old_start = u64::from(gap.offset);
    let old_end = old_start
      .checked_add(bytes.len() as u64)
      .ok_or_else(|| CodecError::Message("DARC gap range overflows".to_owned()))?;
    let recorded_end = u64::from(metadata.data_offset) + u64::from(metadata.data_size);
    if old_start < u64::from(metadata.data_offset) || old_end > recorded_end {
      return Err(CodecError::Message(
        "DARC non-zero padding is outside the recorded data area".to_owned(),
      ));
    }

    let next = old_files.iter().find(|(start, _, _)| *start >= old_end);
    let relocated_start = if let Some((next_start, _, index)) = next {
      let distance = next_start.checked_sub(old_start).ok_or_else(|| {
        CodecError::Message("DARC gap is not before its following file".to_owned())
      })?;
      u64::from(offsets[*index])
        .checked_sub(distance)
        .ok_or_else(|| {
          CodecError::Message("DARC gap cannot be relocated before its following file".to_owned())
        })?
    } else if let Some((_, previous_end, index)) =
      old_files.iter().rev().find(|(_, end, _)| *end <= old_start)
    {
      let distance = old_start.checked_sub(*previous_end).ok_or_else(|| {
        CodecError::Message("DARC gap is not after its preceding file".to_owned())
      })?;
      u64::from(offsets[*index])
        .checked_add(contents[*index].len() as u64)
        .and_then(|end| end.checked_add(distance))
        .ok_or_else(|| CodecError::Message("DARC gap relocation overflows".to_owned()))?
    } else {
      return Err(CodecError::Message(
        "DARC non-zero padding has no file boundary to preserve during relayout".to_owned(),
      ));
    };
    let relocated_end = relocated_start
      .checked_add(bytes.len() as u64)
      .ok_or_else(|| CodecError::Message("DARC gap relocation overflows".to_owned()))?;
    if relocated_start < data_offset
      || new_files
        .iter()
        .any(|(start, end)| relocated_start < *end && *start < relocated_end)
      || relocated.iter().any(|(start, other): &(u64, Vec<u8>)| {
        relocated_start < *start + other.len() as u64 && *start < relocated_end
      })
    {
      return Err(CodecError::Message(
        "DARC non-zero padding cannot be preserved without overlapping relaid-out data".to_owned(),
      ));
    }
    relocated.push((relocated_start, bytes));
  }
  Ok(relocated)
}

fn name_matches(name_area: &[u8], offset: u32, raw: &[u8], endian: Endian) -> bool {
  let offset = offset as usize;
  let Some(name) = name_area.get(offset..offset + raw.len()) else {
    return false;
  };
  let Some(terminator) = name_area.get(offset + raw.len()..offset + raw.len() + 2) else {
    return false;
  };
  name == raw && endian.u16_bytes(0) == terminator
}

fn push_name(name_area: &mut Vec<u8>, raw: &[u8], endian: Endian) -> Result<u32, CodecError> {
  let offset = u32::try_from(name_area.len())
    .map_err(|_| CodecError::Message("DARC name area is impossibly large".to_owned()))?;
  if offset > NAME_MASK {
    return Err(CodecError::Message(format!(
      "DARC name offset {offset:#x} does not fit the 24-bit field"
    )));
  }
  name_area.extend_from_slice(raw);
  name_area.extend_from_slice(&endian.u16_bytes(0));
  Ok(offset)
}

fn metadata_file_path(entry: &EntryMetadata) -> String {
  join_path(&entry.parent, &entry.name)
}

fn match_artifacts<'a>(
  entries: &[&EntryMetadata],
  artifacts: &'a [DecodedArtifact],
) -> Result<Vec<&'a [u8]>, CodecError> {
  let mut used = HashSet::with_capacity(artifacts.len());
  let mut contents = Vec::with_capacity(entries.len());
  for entry in entries {
    let path = metadata_file_path(entry);
    let expected = format!("{FILES_PREFIX}{path}");
    let matching: Vec<&DecodedArtifact> = artifacts
      .iter()
      .filter(|artifact| artifact.name == expected)
      .collect();
    match matching.as_slice() {
      [] => {
        return Err(CodecError::Message(format!(
          "Expected a {expected} artifact, found none"
        )));
      }
      [artifact] => {
        if !used.insert(artifact.name.as_str()) {
          return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
        }
        contents.push(artifact.data.as_slice());
      }
      _ => return Err(CodecError::DuplicateArtifact(expected)),
    }
  }
  for artifact in artifacts {
    if !used.contains(artifact.name.as_str()) {
      return Err(CodecError::Message(format!(
        "Unexpected artifact: {}",
        artifact.name
      )));
    }
  }
  Ok(contents)
}

fn prefix_bytes(text: &str, length: usize, what: &str) -> Result<Vec<u8>, CodecError> {
  let bytes = decode_hex(text, what)?;
  if bytes.is_empty() {
    return Ok(vec![0; length]);
  }
  if bytes.len() != length {
    return Err(CodecError::Message(format!(
      "DARC metadata {what} is {} bytes, expected {length}",
      bytes.len()
    )));
  }
  Ok(bytes)
}

impl Codec for Darc {
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
    match parse(data) {
      Ok(parsed) => match file_ranges(&parsed) {
        Ok(_) => Ok(CodecHandlingConfidence::Certain),
        Err(_) => Ok(CodecHandlingConfidence::No),
      },
      Err(_) => Ok(CodecHandlingConfidence::No),
    }
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("darc".to_owned())
  }

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding Nintendo DARC archive");
    let _args = DarcCodecArgs::from_args(args)?;
    let parsed = parse(data)?;
    let mut ranges = file_ranges(&parsed)?;
    let header = &parsed.header;

    let name_area = data
      .get(parsed.name_area_start..parsed.name_area_start + parsed.name_area_len)
      .ok_or_else(|| CodecError::Message("DARC name area exceeds the input".to_owned()))?;
    let table_end = header.table_offset as usize + header.table_size as usize;
    let data_offset = header.data_offset as usize;
    let file_size = header.file_size as usize;

    let mut entries = Vec::with_capacity(parsed.entries.len());
    let mut artifacts = Vec::with_capacity(parsed.entries.len() + 1);
    let first = if parsed.dot_entry { 2 } else { 1 };
    let root_name_offset = parsed.entries[0].name_offset;
    let dot_name_offset = parsed.dot_entry.then(|| parsed.entries[1].name_offset);

    for index in first..parsed.entries.len() {
      let entry = &parsed.entries[index];
      let path = &parsed.paths[index];
      if entry.is_folder {
        entries.push(EntryMetadata {
          kind: EntryKind::Folder,
          parent: parent_path(path).to_owned(),
          name: entry.name.clone(),
          name_raw: entry.name_raw.clone(),
          name_offset: Some(entry.name_offset),
          data_offset: None,
          size: None,
          data_alignment: None,
        });
        continue;
      }

      let start = entry.second as usize;
      let size = entry.third as usize;
      let end = start
        .checked_add(size)
        .ok_or_else(|| CodecError::Message("DARC file range overflows".to_owned()))?;
      let bytes = data.get(start..end).ok_or_else(|| {
        CodecError::Message(format!(
          "DARC file {path:?} range {start:#x}..{end:#x} exceeds the input"
        ))
      })?;
      entries.push(EntryMetadata {
        kind: EntryKind::File,
        parent: parent_path(path).to_owned(),
        name: entry.name.clone(),
        name_raw: entry.name_raw.clone(),
        name_offset: Some(entry.name_offset),
        data_offset: Some(entry.second),
        size: Some(entry.third),
        data_alignment: None,
      });
      artifacts.push(DecodedArtifact {
        name: format!("{FILES_PREFIX}{path}"),
        data: bytes.to_vec(),
      });
    }

    let prefix = &data[HEADER_SIZE..header.table_offset as usize];
    let table_padding = &data[table_end..data_offset];
    let (trailing, trailing_zeros) = split_trailing(&data[file_size..]);
    let inferred_alignments = infer_data_alignments(&entries, u64::from(header.data_offset));
    for (entry, alignment) in entries.iter_mut().zip(inferred_alignments) {
      entry.data_alignment = alignment;
    }
    let metadata = DarcMetadata {
      endian: header.endian,
      header_size: header.header_size,
      version: header.version,
      table_offset: header.table_offset,
      data_offset: header.data_offset,
      prefix: hex_or_empty(prefix),
      name_area: hex::encode(name_area),
      table_padding: hex_or_empty(table_padding),
      data_size: header.file_size - header.data_offset,
      gaps: collect_gaps(data, data_offset, file_size, &mut ranges)?,
      trailing,
      trailing_zeros,
      dot_entry: parsed.dot_entry,
      root_name_offset,
      dot_name_offset,
      entries,
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

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"encode"))
  )]
  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding Nintendo DARC archive");
    let _args = DarcCodecArgs::from_args(args)?;

    let (metadata_artifact, file_artifacts) = artifacts.split_first().ok_or_else(|| {
      CodecError::Message(format!("Expected a {METADATA_NAME} artifact, found none"))
    })?;
    if metadata_artifact.name != METADATA_NAME {
      return Err(CodecError::Message(format!(
        "Expected the first artifact to be {METADATA_NAME}, found {}",
        metadata_artifact.name
      )));
    }
    let metadata = DarcMetadata::from_json(&metadata_artifact.data)?;
    let endian = metadata.endian;

    if usize::from(metadata.header_size) < HEADER_SIZE {
      return Err(CodecError::Message(format!(
        "DARC metadata header size is {}, expected at least {HEADER_SIZE}",
        metadata.header_size
      )));
    }
    if metadata.table_offset < u32::from(metadata.header_size) {
      return Err(CodecError::Message(format!(
        "DARC metadata table offset {:#x} is before the end of its header ({:#x})",
        metadata.table_offset, metadata.header_size
      )));
    }

    let dot = metadata.dot_entry;
    let first = 1 + usize::from(dot);
    let count = first + metadata.entries.len();
    let count_u32 = u32::try_from(count)
      .map_err(|_| CodecError::Message("DARC entry count does not fit the format".to_owned()))?;

    // Validate names and build the folder paths before resolving parents.
    let mut folder_paths: HashMap<String, usize> = HashMap::new();
    // Top-level entries hang off the "." entry when there is one, which is the
    // real root and sits right after the synthetic one.
    folder_paths.insert(String::new(), usize::from(dot));
    let mut paths = Vec::with_capacity(metadata.entries.len());
    for (index, entry) in metadata.entries.iter().enumerate() {
      validate_component(&entry.name)?;
      let path = join_path(&entry.parent, &entry.name);
      if entry.kind == EntryKind::Folder
        && folder_paths.insert(path.clone(), first + index).is_some()
      {
        return Err(CodecError::Message(format!(
          "DARC metadata describes folder {path:?} twice"
        )));
      }
      paths.push(path);
    }

    // Resolve parents, reject cycles/forward references and duplicate paths.
    let mut parents = Vec::with_capacity(metadata.entries.len());
    let mut seen = HashSet::with_capacity(metadata.entries.len());
    for (index, entry) in metadata.entries.iter().enumerate() {
      let parent_index = *folder_paths.get(&entry.parent).ok_or_else(|| {
        CodecError::Message(format!(
          "DARC entry {:?} has unknown parent folder {:?}",
          entry.name, entry.parent
        ))
      })?;
      if parent_index >= first + index {
        return Err(CodecError::Message(format!(
          "DARC entry {:?} is listed before its parent folder {:?}",
          entry.name, entry.parent
        )));
      }
      if !seen.insert(paths[index].clone()) {
        return Err(CodecError::Message(format!(
          "DARC metadata describes path {:?} twice",
          paths[index]
        )));
      }
      parents.push(parent_index);
    }

    // A folder's end index is the first entry after its contiguous subtree.
    let mut folder_ends = vec![None; metadata.entries.len()];
    for (index, entry) in metadata.entries.iter().enumerate() {
      if entry.kind != EntryKind::Folder {
        continue;
      }
      let path = &paths[index];
      let mut last = None;
      for (other, candidate) in metadata.entries.iter().enumerate().skip(index + 1) {
        if is_descendant(&candidate.parent, path) {
          last = Some(other);
        }
      }
      let end = match last {
        Some(last) => {
          for candidate in &metadata.entries[index + 1..=last] {
            if !is_descendant(&candidate.parent, path) {
              return Err(CodecError::Message(format!(
                "DARC folder {path:?} has non-contiguous children"
              )));
            }
          }
          first + last + 1
        }
        None => first + index + 1,
      };
      folder_ends[index] = Some(end);
    }

    // Match the artifacts to the files, in physical table order.
    let file_entries: Vec<&EntryMetadata> = metadata
      .entries
      .iter()
      .filter(|entry| entry.kind == EntryKind::File)
      .collect();
    let contents = match_artifacts(&file_entries, file_artifacts)?;
    let file_paths: Vec<String> = file_entries
      .iter()
      .map(|entry| metadata_file_path(entry))
      .collect();
    validate_file_paths(file_paths.iter().map(String::as_str))?;

    // Names: reuse the decoded area verbatim when every name still matches.
    let raw_names: Vec<Vec<u8>> = metadata
      .entries
      .iter()
      .map(|entry| encode_name(&entry.name, entry.name_raw.as_deref(), endian, "entry"))
      .collect::<Result<_, _>>()?;
    let mut name_area = decode_hex(&metadata.name_area, "name area")?;
    let mut root_name_offset = metadata.root_name_offset;
    let mut dot_name_offset = metadata.dot_name_offset;
    // "." must be stored as UTF-16 like every other DARC name.
    let dot_raw = endian.utf16_bytes([u16::from(b'.')]);
    let mut name_offsets: Vec<u32> = Vec::with_capacity(metadata.entries.len());
    let mut reuse_names = name_matches(&name_area, root_name_offset, &[], endian);
    if reuse_names && dot {
      reuse_names =
        dot_name_offset.is_some_and(|offset| name_matches(&name_area, offset, &dot_raw, endian));
    }
    if reuse_names {
      for (entry, raw) in metadata.entries.iter().zip(&raw_names) {
        match entry.name_offset {
          Some(offset) if name_matches(&name_area, offset, raw, endian) => {
            name_offsets.push(offset);
          }
          _ => {
            reuse_names = false;
            break;
          }
        }
      }
    }
    if !reuse_names {
      debug!("DARC name area rebuilt: an entry was added, removed or renamed");
      name_area.clear();
      name_offsets.clear();
      root_name_offset = push_name(&mut name_area, &[], endian)?;
      dot_name_offset = if dot {
        Some(push_name(&mut name_area, &dot_raw, endian)?)
      } else {
        None
      };
      for raw in &raw_names {
        name_offsets.push(push_name(&mut name_area, raw, endian)?);
      }
    }

    let table_size = count
      .checked_mul(ENTRY_SIZE)
      .and_then(|size| size.checked_add(name_area.len()))
      .ok_or_else(|| CodecError::Message("DARC table size overflows".to_owned()))?;
    let table_offset = metadata.table_offset as usize;
    let table_end = table_offset
      .checked_add(table_size)
      .ok_or_else(|| CodecError::Message("DARC table range overflows".to_owned()))?;

    // Keep the decoded data offset while the table still fits before it.
    let recorded_data_offset = metadata.data_offset as usize;
    let reuse_padding = table_end <= recorded_data_offset;
    let data_offset = if reuse_padding {
      recorded_data_offset
    } else {
      align_up(table_end as u64, FALLBACK_DATA_ALIGN)
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or_else(|| CodecError::Message("DARC data offset overflows".to_owned()))?
    };
    if data_offset < table_end {
      return Err(CodecError::Message(
        "DARC data area would overlap its table".to_owned(),
      ));
    }

    let prefix = prefix_bytes(
      &metadata.prefix,
      table_offset - HEADER_SIZE,
      "header prefix",
    )?;
    let padding_len = data_offset - table_end;
    let mut table_padding = vec![0; padding_len];
    if reuse_padding {
      // The recorded padding only applies while the table keeps its size. When
      // an entry is added, removed or renamed the padding is resized instead.
      let recorded = decode_hex(&metadata.table_padding, "table padding")?;
      if recorded.len() == padding_len {
        table_padding.copy_from_slice(&recorded);
      } else if !recorded.is_empty() {
        debug!("DARC table padding resized: the recorded padding no longer fits");
      }
    }

    let data_offset_u32 = u32::try_from(data_offset)
      .map_err(|_| CodecError::Message("DARC data offset does not fit the header".to_owned()))?;

    // Reuse file offsets while every file still has the recorded size and the
    // ranges fit the decoded data area; otherwise lay the data out again.
    let recorded_end = u64::from(metadata.data_offset)
      .checked_add(u64::from(metadata.data_size))
      .ok_or_else(|| CodecError::Message("DARC data area overflows".to_owned()))?;
    let inferred_alignments =
      infer_data_alignments(&metadata.entries, u64::from(metadata.data_offset));
    let file_indices: Vec<usize> = metadata
      .entries
      .iter()
      .enumerate()
      .filter_map(|(index, entry)| (entry.kind == EntryKind::File).then_some(index))
      .collect();
    let alignments: Vec<u64> = file_entries
      .iter()
      .zip(&file_indices)
      .map(|(entry, &index)| {
        let alignment = entry
          .data_alignment
          .map(u64::from)
          .or_else(|| inferred_alignments[index].map(u64::from))
          .unwrap_or(FALLBACK_DATA_ALIGN);
        if alignment < FALLBACK_DATA_ALIGN || !alignment.is_power_of_two() {
          return Err(CodecError::Message(format!(
            "DARC file {:?} has invalid data alignment {alignment:#x}",
            entry.name
          )));
        }
        Ok(alignment)
      })
      .collect::<Result<_, _>>()?;
    let mut file_offsets = Vec::with_capacity(file_entries.len());
    let mut reuse_layout = data_offset as u64 <= recorded_end;
    if reuse_layout {
      for entry in &file_entries {
        let Some(offset) = entry.data_offset else {
          reuse_layout = false;
          break;
        };
        if offset < data_offset_u32 {
          reuse_layout = false;
          break;
        }
        file_offsets.push(offset);
      }
    }
    if reuse_layout {
      let mut ranges: Vec<(u64, u64)> = file_offsets
        .iter()
        .zip(&contents)
        .filter(|(_, bytes)| !bytes.is_empty())
        .map(|(&offset, bytes)| {
          let start = u64::from(offset);
          (start, start + bytes.len() as u64)
        })
        .collect();
      ranges.sort_unstable();
      for pair in ranges.windows(2) {
        if pair[1].0 < pair[0].1 {
          reuse_layout = false;
          break;
        }
      }
      let mut gap_ranges = Vec::with_capacity(metadata.gaps.len());
      for gap in &metadata.gaps {
        let bytes = decode_hex(&gap.data, "gap")?;
        let start = u64::from(gap.offset);
        let end = start
          .checked_add(bytes.len() as u64)
          .ok_or_else(|| CodecError::Message("DARC gap range overflows".to_owned()))?;
        if gap.offset < data_offset_u32 || end > recorded_end {
          reuse_layout = false;
          break;
        }
        gap_ranges.push((start, end));
      }
      if reuse_layout {
        for (gap_start, gap_end) in &gap_ranges {
          for (file_start, file_end) in &ranges {
            if gap_start < file_end && file_start < gap_end {
              reuse_layout = false;
              break;
            }
          }
          if !reuse_layout {
            break;
          }
        }
      }
    }

    let mut relocated_gaps = Vec::new();
    let data_size = if reuse_layout {
      let end = file_offsets
        .iter()
        .zip(&contents)
        .map(|(offset, bytes)| u64::from(*offset) + bytes.len() as u64)
        .max()
        .unwrap_or(data_offset as u64)
        .max(recorded_end);
      u32::try_from(end - data_offset as u64)
        .map_err(|_| CodecError::Message("DARC data area does not fit the format".to_owned()))?
    } else {
      debug!("DARC data layout recomputed: the decoded offsets no longer fit");
      file_offsets.resize(file_entries.len(), 0);
      let mut order: Vec<usize> = (0..file_entries.len()).collect();
      order.sort_by_key(|&index| file_entries[index].data_offset.unwrap_or(u32::MAX));
      let mut cursor = data_offset as u64;
      for index in order {
        let offset = align_up(cursor, alignments[index])
          .ok_or_else(|| CodecError::Message("DARC data area overflows".to_owned()))?;
        file_offsets[index] = u32::try_from(offset).map_err(|_| {
          CodecError::Message("DARC file offset does not fit the format".to_owned())
        })?;
        cursor = offset
          .checked_add(contents[index].len() as u64)
          .ok_or_else(|| CodecError::Message("DARC data area overflows".to_owned()))?;
      }
      relocated_gaps = relocate_gaps(
        &metadata,
        &file_entries,
        &contents,
        &file_offsets,
        data_offset as u64,
      )?;
      for (offset, bytes) in &relocated_gaps {
        cursor = cursor.max(*offset + bytes.len() as u64);
      }
      u32::try_from(cursor - data_offset as u64)
        .map_err(|_| CodecError::Message("DARC data area does not fit the format".to_owned()))?
    };

    let mut table = Vec::with_capacity(count * ENTRY_SIZE);
    let write_entry =
      |table: &mut Vec<u8>, name_offset: u32, is_folder: bool, second: u32, third: u32| {
        let name_field = if is_folder {
          name_offset | FOLDER_FLAG
        } else {
          name_offset
        };
        table.extend_from_slice(&endian.u32_bytes(name_field));
        table.extend_from_slice(&endian.u32_bytes(second));
        table.extend_from_slice(&endian.u32_bytes(third));
      };
    write_entry(&mut table, root_name_offset, true, 0, count_u32);
    if dot {
      let offset = dot_name_offset.ok_or_else(|| {
        CodecError::Message("DARC metadata has a '.' entry but no name offset".to_owned())
      })?;
      write_entry(&mut table, offset, true, 0, count_u32);
    }
    let mut file_cursor = 0usize;
    for (index, entry) in metadata.entries.iter().enumerate() {
      let name_offset = name_offsets[index];
      if name_offset > NAME_MASK {
        return Err(CodecError::Message(format!(
          "DARC name offset {name_offset:#x} does not fit the 24-bit field"
        )));
      }
      match entry.kind {
        EntryKind::Folder => {
          let end = folder_ends[index].ok_or_else(|| {
            CodecError::Message(format!("DARC folder {:?} has no end index", entry.name))
          })?;
          write_entry(
            &mut table,
            name_offset,
            true,
            u32::try_from(parents[index]).map_err(|_| {
              CodecError::Message("DARC folder parent index does not fit".to_owned())
            })?,
            u32::try_from(end)
              .map_err(|_| CodecError::Message("DARC folder end does not fit".to_owned()))?,
          );
        }
        EntryKind::File => {
          let bytes = contents[file_cursor];
          let offset = file_offsets[file_cursor];
          file_cursor += 1;
          let size = u32::try_from(bytes.len())
            .map_err(|_| CodecError::Message("DARC file does not fit the format".to_owned()))?;
          write_entry(&mut table, name_offset, false, offset, size);
        }
      }
    }

    let mut data = vec![0u8; data_size as usize];
    if reuse_layout {
      write_gaps(&mut data, data_offset_u32, &metadata.gaps)?;
    } else {
      for (offset, bytes) in relocated_gaps {
        let start = usize::try_from(offset - u64::from(data_offset_u32))
          .map_err(|_| CodecError::Message("DARC gap offset overflows".to_owned()))?;
        let end = start
          .checked_add(bytes.len())
          .ok_or_else(|| CodecError::Message("DARC gap range overflows".to_owned()))?;
        data
          .get_mut(start..end)
          .ok_or_else(|| CodecError::Message("DARC gap exceeds the data area".to_owned()))?
          .copy_from_slice(&bytes);
      }
    }
    for (offset, bytes) in file_offsets.iter().zip(&contents) {
      let start = (*offset as usize).checked_sub(data_offset).ok_or_else(|| {
        CodecError::Message("DARC file offset is before the data area".to_owned())
      })?;
      let end = start
        .checked_add(bytes.len())
        .ok_or_else(|| CodecError::Message("DARC file range overflows".to_owned()))?;
      data
        .get_mut(start..end)
        .ok_or_else(|| CodecError::Message("DARC file range exceeds the data area".to_owned()))?
        .copy_from_slice(bytes);
    }

    let file_size = data_offset
      .checked_add(data_size as usize)
      .ok_or_else(|| CodecError::Message("DARC file size overflows".to_owned()))?;
    let file_size_u32 = u32::try_from(file_size)
      .map_err(|_| CodecError::Message("DARC file size does not fit the header".to_owned()))?;
    let table_size_u32 = u32::try_from(table_size)
      .map_err(|_| CodecError::Message("DARC table size does not fit the header".to_owned()))?;
    let trailing = join_trailing(&metadata.trailing, metadata.trailing_zeros)?;

    let mut out = Vec::with_capacity(file_size + trailing.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&endian.u16_bytes(0xFEFF));
    out.extend_from_slice(&endian.u16_bytes(metadata.header_size));
    out.extend_from_slice(&endian.u32_bytes(metadata.version));
    out.extend_from_slice(&endian.u32_bytes(file_size_u32));
    out.extend_from_slice(&endian.u32_bytes(metadata.table_offset));
    out.extend_from_slice(&endian.u32_bytes(table_size_u32));
    out.extend_from_slice(&endian.u32_bytes(data_offset_u32));
    debug_assert_eq!(out.len(), HEADER_SIZE);
    out.extend_from_slice(&prefix);
    out.extend_from_slice(&table);
    out.extend_from_slice(&name_area);
    out.extend_from_slice(&table_padding);
    out.extend_from_slice(&data);
    out.extend_from_slice(&trailing);
    Ok(out)
  }
}

fn hex_or_empty(bytes: &[u8]) -> String {
  if bytes.iter().all(|&byte| byte == 0) {
    String::new()
  } else {
    hex::encode(bytes)
  }
}
