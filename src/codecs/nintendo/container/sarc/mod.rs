//! Nintendo SARC (3DS archive) codec.
//!
//! SARC is the archive format used by the 3DS eShop applet and by many first-
//! party titles. It is a flat list of named files: an SFAT hash table, an SFNT
//! name table and a data area whose entries are addressed relative to the
//! header's data offset. Nodes are normally sorted by filename hash because
//! games search them with a binary search.
//!
//! Format references:
//! - <https://www.3dbrew.org/wiki/SARC>
//! - <https://problemkaputt.de/gbatek.htm#3dsfilesarchivesarc>
//!
//! The rebuild preserves the decoded node order, name table and data offsets
//! while they still describe the artifacts, so an unmodified rebuild is
//! byte-identical. When a name changes the hashes are recomputed and the nodes
//! are sorted by hash; when a file changes size the data area is laid out again
//! on 0x80-byte boundaries.

use super::super::n3ds::common::align_up;
use super::{
  Endian, FILES_PREFIX, Gap, collect_gaps, decode_hex, is_zero_u64, join_trailing, split_trailing,
  validate_file_paths, validate_relative_path, write_gaps,
};
use crate::codecs::errors::CodecError;
use crate::codecs::{Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact, LudaredCodecArgs};
use log::{debug, trace};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[cfg(test)]
mod tests;

const CODEC_ID: &str = "std/nintendo/container/sarc";
const CODEC_NAME: &str = "Nintendo SARC archive";
const CODEC_DESC: &str = r#"Nintendo SARC archive

Extracts a SARC archive into:
- SARC.json: header fields, node hashes and attributes, the raw name table and
  the file layout, including non-zero padding, so an unmodified rebuild stays
  byte-identical.
- One artifact per file, under files/ with its full path. Names may contain
  slashes and therefore describe a directory tree.

Nodes are normally sorted by filename hash. Rebuilding preserves that order,
the stored hashes and the data offsets while they still describe the artifacts.
When a name changes the hashes are recomputed and the nodes are re-sorted by
hash; when a file changes size the data area is laid out again on 0x80-byte
boundaries. Unnamed nodes keep their hash and are exposed under a synthetic
artifact name recorded in SARC.json.

No arguments.
"#;

const MAGIC: &[u8; 4] = b"SARC";
const SFAT_MAGIC: &[u8; 4] = b"SFAT";
const SFNT_MAGIC: &[u8; 4] = b"SFNT";
const HEADER_SIZE: usize = 0x14;
const SFAT_HEADER_SIZE: usize = 0x0C;
const SFNT_HEADER_SIZE: usize = 0x08;
const NODE_SIZE: usize = 0x10;
const HAS_NAME: u32 = 0x0100_0000;
const NAME_MASK: u32 = 0x00FF_FFFF;
const DATA_ALIGN: u64 = 0x80;
const METADATA_NAME: &str = "SARC.json";

pub(crate) struct Sarc;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SarcCodecArgs {}

impl LudaredCodecArgs for SarcCodecArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeMetadata {
  /// UTF-8 view of the name; absent when the node has no name.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  name: Option<String>,
  /// Raw name bytes when they are not valid UTF-8.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  name_raw: Option<String>,
  /// Synthetic artifact name used for an unnamed node.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  artifact: Option<String>,
  hash: u32,
  attributes: u32,
  /// Byte offset within the name table, as decoded.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  name_offset: Option<u32>,
  /// Data start relative to the header's data offset.
  data_start: u32,
  /// Data end relative to the header's data offset.
  data_end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SarcMetadata {
  endian: Endian,
  header_size: u16,
  version: u16,
  reserved: u16,
  data_offset: u32,
  /// Bytes between the fixed header and the SFAT chunk; empty when all zero.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  header_extra: String,
  hash_multiplier: u32,
  sfat_header_size: u16,
  /// Bytes between the fixed SFAT header and its nodes; empty when all zero.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  sfat_extra: String,
  sfnt_header_size: u16,
  sfnt_reserved: u16,
  /// Bytes between the fixed SFNT header and the names; empty when all zero.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  sfnt_extra: String,
  /// Raw name table, from the end of the SFNT header to the data offset.
  #[serde(default)]
  name_table: String,
  /// Length of the data area.
  data_size: u32,
  /// Non-zero bytes between file data.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  gaps: Vec<Gap>,
  /// Bytes after the archive's declared total size, when any is non-zero.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  trailing: String,
  /// Length of the zero padding after the declared total size.
  #[serde(default, skip_serializing_if = "is_zero_u64")]
  trailing_zeros: u64,
  /// Nodes in table order.
  nodes: Vec<NodeMetadata>,
}

impl SarcMetadata {
  fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the SARC metadata: {err}")))
  }

  fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the SARC metadata artifact: {err}"))
    })
  }
}

/// The fixed archive header, before the SFAT chunk.
#[derive(Debug)]
struct ArchiveHeader {
  endian: Endian,
  /// Size of this header, in bytes.
  header_size: u16,
  version: u16,
  reserved: u16,
  total_size: u32,
  data_offset: u32,
  hash_multiplier: u32,
  sfat_header_size: u16,
  sfnt_header_size: u16,
  sfnt_reserved: u16,
  node_count: u16,
  sfat_offset: usize,
  sfnt_offset: usize,
  names_start: usize,
}

#[derive(Debug)]
struct RawNode {
  hash: u32,
  attributes: u32,
  name_offset: Option<u32>,
  name: Option<String>,
  name_raw: Option<String>,
  data_start: u32,
  data_end: u32,
}

struct ParsedSarc {
  header: ArchiveHeader,
  nodes: Vec<RawNode>,
  name_table_start: usize,
  name_table_len: usize,
}

fn parse_header(data: &[u8]) -> Result<ArchiveHeader, CodecError> {
  let header = data.get(..HEADER_SIZE).ok_or_else(|| {
    CodecError::Message(format!(
      "SARC header is truncated: {} bytes (minimum: {HEADER_SIZE})",
      data.len()
    ))
  })?;
  if header[..4] != MAGIC[..] {
    return Err(CodecError::Message(
      "Not a SARC archive: the 'SARC' magic is missing".to_owned(),
    ));
  }
  let endian = Endian::from_bom(header, 6)?;
  let header_size = endian.read_u16(header, 0x04)?;
  let total_size = endian.read_u32(header, 0x08)?;
  let data_offset = endian.read_u32(header, 0x0C)?;
  let version = endian.read_u16(header, 0x10)?;
  let reserved = endian.read_u16(header, 0x12)?;

  if usize::from(header_size) < HEADER_SIZE {
    return Err(CodecError::Message(format!(
      "SARC header is {header_size} bytes, expected at least {HEADER_SIZE}"
    )));
  }
  if total_size as usize > data.len() {
    return Err(CodecError::Message(format!(
      "SARC declares {} bytes but the input is only {} bytes",
      total_size,
      data.len()
    )));
  }
  if data_offset > total_size {
    return Err(CodecError::Message(format!(
      "SARC data offset {data_offset:#x} exceeds its total size {total_size:#x}"
    )));
  }
  let sfat_offset = header_size as usize;
  let sfat = data
    .get(sfat_offset..)
    .ok_or_else(|| CodecError::Message("SARC SFAT chunk is outside the archive".to_owned()))?;
  if sfat.get(..4) != Some(SFAT_MAGIC) {
    return Err(CodecError::Message(
      "Not a SARC archive: the 'SFAT' magic is missing".to_owned(),
    ));
  }
  let sfat_header_size = endian.read_u16(sfat, 0x04)?;
  let node_count = endian.read_u16(sfat, 0x06)?;
  let hash_multiplier = endian.read_u32(sfat, 0x08)?;
  if usize::from(sfat_header_size) < SFAT_HEADER_SIZE {
    return Err(CodecError::Message(format!(
      "SARC SFAT header is {sfat_header_size} bytes, expected at least {SFAT_HEADER_SIZE}"
    )));
  }
  let nodes_start = sfat_offset
    .checked_add(sfat_header_size as usize)
    .ok_or_else(|| CodecError::Message("SARC SFAT range overflows".to_owned()))?;
  let nodes_end = nodes_start
    .checked_add(node_count as usize * NODE_SIZE)
    .ok_or_else(|| CodecError::Message("SARC node table range overflows".to_owned()))?;
  let sfnt_offset = nodes_end;
  let sfnt = data
    .get(sfnt_offset..)
    .ok_or_else(|| CodecError::Message("SARC SFNT chunk is outside the archive".to_owned()))?;
  if sfnt.get(..4) != Some(SFNT_MAGIC) {
    return Err(CodecError::Message(
      "Not a SARC archive: the 'SFNT' magic is missing".to_owned(),
    ));
  }
  let sfnt_header_size = endian.read_u16(sfnt, 0x04)?;
  let sfnt_reserved = endian.read_u16(sfnt, 0x06)?;
  if usize::from(sfnt_header_size) < SFNT_HEADER_SIZE {
    return Err(CodecError::Message(format!(
      "SARC SFNT header is {sfnt_header_size} bytes, expected at least {SFNT_HEADER_SIZE}"
    )));
  }
  let names_start = sfnt_offset
    .checked_add(sfnt_header_size as usize)
    .ok_or_else(|| CodecError::Message("SARC SFNT range overflows".to_owned()))?;
  if names_start > data_offset as usize {
    return Err(CodecError::Message(format!(
      "SARC name table starts at {names_start:#x}, after the data offset {data_offset:#x}"
    )));
  }

  Ok(ArchiveHeader {
    endian,
    header_size,
    version,
    reserved,
    total_size,
    data_offset,
    hash_multiplier,
    sfat_header_size,
    sfnt_header_size,
    sfnt_reserved,
    node_count,
    sfat_offset,
    sfnt_offset,
    names_start,
  })
}

fn read_cstr(table: &[u8], offset: usize) -> Result<&[u8], CodecError> {
  let tail = table.get(offset..).ok_or_else(|| {
    CodecError::Message(format!(
      "SARC name offset {offset:#x} exceeds the name table ({} bytes)",
      table.len()
    ))
  })?;
  let end = tail.iter().position(|&byte| byte == 0).ok_or_else(|| {
    CodecError::Message(format!("SARC name at {offset:#x} is not NUL-terminated"))
  })?;
  Ok(&tail[..end])
}

fn decode_name(raw: &[u8]) -> (String, Option<String>) {
  match std::str::from_utf8(raw) {
    Ok(name) => (name.to_owned(), None),
    Err(_) => (
      format!("_sarc_raw_{}", hex::encode(raw)),
      Some(hex::encode(raw)),
    ),
  }
}

fn encode_name(name: &str, name_raw: Option<&str>) -> Result<Vec<u8>, CodecError> {
  let raw = match name_raw {
    Some(raw) => decode_hex(raw, "node name")?,
    None => name.as_bytes().to_vec(),
  };
  if raw.contains(&0) {
    return Err(CodecError::Message(format!(
      "SARC metadata name {name:?} contains a NUL byte"
    )));
  }
  let (decoded, decoded_raw) = decode_name(&raw);
  if decoded != name || decoded_raw.as_deref() != name_raw {
    return Err(CodecError::Message(format!(
      "SARC metadata name {name:?} does not match its raw name"
    )));
  }
  Ok(raw)
}

fn parse(data: &[u8]) -> Result<ParsedSarc, CodecError> {
  let header = parse_header(data)?;
  let name_table_start = header.names_start;
  let name_table_len = header.data_offset as usize - name_table_start;
  let name_table = data
    .get(name_table_start..name_table_start + name_table_len)
    .ok_or_else(|| CodecError::Message("SARC name table exceeds the input".to_owned()))?;

  let nodes_start = header.sfat_offset + header.sfat_header_size as usize;
  let node_count = header.node_count as usize;
  let data_size = header.total_size - header.data_offset;
  let mut nodes = Vec::with_capacity(node_count);
  let mut ranges = Vec::new();
  for index in 0..node_count {
    let base = nodes_start + index * NODE_SIZE;
    let hash = header.endian.read_u32(data, base)?;
    let attributes = header.endian.read_u32(data, base + 0x4)?;
    let data_start = header.endian.read_u32(data, base + 0x8)?;
    let data_end = header.endian.read_u32(data, base + 0xC)?;
    if data_start > data_end {
      return Err(CodecError::Message(format!(
        "SARC node {index} has data start {data_start:#x} after its end {data_end:#x}"
      )));
    }
    if data_end > data_size {
      return Err(CodecError::Message(format!(
        "SARC node {index} data ends at {data_end:#x}, past the {data_size:#x}-byte data area"
      )));
    }
    if data_end > data_start {
      ranges.push((data_start as usize, data_end as usize));
    }

    let has_name = attributes & HAS_NAME != 0;
    let (name_offset, name, name_raw) = if has_name {
      let words = attributes & NAME_MASK;
      let offset = words
        .checked_mul(4)
        .ok_or_else(|| CodecError::Message(format!("SARC node {index} name offset overflows")))?;
      let raw = read_cstr(name_table, offset as usize)?;
      let (name, name_raw) = decode_name(raw);
      (Some(offset), Some(name), name_raw)
    } else {
      (None, None, None)
    };
    nodes.push(RawNode {
      hash,
      attributes,
      name_offset,
      name,
      name_raw,
      data_start,
      data_end,
    });
  }

  ranges.sort_unstable();
  for pair in ranges.windows(2) {
    if pair[1].0 < pair[0].1 {
      return Err(CodecError::Message(format!(
        "SARC node data ranges {:#x}..{:#x} and {:#x}..{:#x} overlap",
        pair[0].0, pair[0].1, pair[1].0, pair[1].1
      )));
    }
  }

  Ok(ParsedSarc {
    header,
    nodes,
    name_table_start,
    name_table_len,
  })
}

fn calc_hash(name: &[u8], multiplier: u32) -> u32 {
  name.iter().fold(0u32, |result, &byte| {
    result
      .wrapping_mul(multiplier)
      .wrapping_add(u32::from(byte))
  })
}

fn name_matches(name_table: &[u8], offset: u32, raw: &[u8]) -> bool {
  let offset = offset as usize;
  if !offset.is_multiple_of(4) {
    return false;
  }
  let Some(name) = name_table.get(offset..offset + raw.len()) else {
    return false;
  };
  name == raw && name_table.get(offset + raw.len()) == Some(&0)
}

fn push_name(name_table: &mut Vec<u8>, raw: &[u8]) -> Result<u32, CodecError> {
  let offset = u32::try_from(name_table.len())
    .map_err(|_| CodecError::Message("SARC name table is impossibly large".to_owned()))?;
  if !offset.is_multiple_of(4) {
    return Err(CodecError::Message(
      "SARC name table offset is not 4-byte aligned".to_owned(),
    ));
  }
  if offset / 4 > NAME_MASK {
    return Err(CodecError::Message(format!(
      "SARC name offset {offset:#x} does not fit the 24-bit field"
    )));
  }
  name_table.extend_from_slice(raw);
  name_table.push(0);
  name_table.resize(name_table.len().next_multiple_of(4), 0);
  Ok(offset)
}

fn hex_or_empty(bytes: &[u8]) -> String {
  if bytes.iter().all(|&byte| byte == 0) {
    String::new()
  } else {
    hex::encode(bytes)
  }
}

fn extra_bytes(text: &str, length: usize, what: &str) -> Result<Vec<u8>, CodecError> {
  let bytes = decode_hex(text, what)?;
  if bytes.is_empty() {
    return Ok(vec![0; length]);
  }
  if bytes.len() != length {
    return Err(CodecError::Message(format!(
      "SARC metadata {what} is {} bytes, expected {length}",
      bytes.len()
    )));
  }
  Ok(bytes)
}

struct WorkNode<'a> {
  meta: &'a NodeMetadata,
  artifact_name: String,
  data: &'a [u8],
  raw_name: Option<Vec<u8>>,
  hash: u32,
  attributes: u32,
  name_offset: Option<u32>,
  data_start: u32,
  data_end: u32,
}

impl Codec for Sarc {
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
      Ok(_) => Ok(CodecHandlingConfidence::Certain),
      Err(_) => Ok(CodecHandlingConfidence::No),
    }
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("sarc".to_owned())
  }

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding Nintendo SARC archive");
    let _args = SarcCodecArgs::from_args(args)?;
    let parsed = parse(data)?;
    let header = &parsed.header;
    let data_offset = header.data_offset as usize;

    let mut artifacts = Vec::with_capacity(parsed.nodes.len() + 1);
    let mut nodes = Vec::with_capacity(parsed.nodes.len());
    let mut artifact_paths = Vec::with_capacity(parsed.nodes.len());
    let mut used_names = HashSet::with_capacity(parsed.nodes.len());
    let mut unnamed_count = 0usize;

    for (index, node) in parsed.nodes.iter().enumerate() {
      let artifact_path = if let Some(name) = &node.name {
        if name.is_empty() {
          return Err(CodecError::Message(format!(
            "SARC node {index} has an empty name, which cannot produce an artifact"
          )));
        }
        validate_relative_path(name, "SARC file")?;
        name.clone()
      } else {
        let mut candidate = format!("_sarc_unnamed_{:08x}", node.hash);
        if !used_names.insert(candidate.clone()) {
          candidate = format!("_sarc_unnamed_{:08x}_{unnamed_count}", node.hash);
          if !used_names.insert(candidate.clone()) {
            return Err(CodecError::DuplicateArtifact(format!(
              "{FILES_PREFIX}{candidate}"
            )));
          }
        }
        unnamed_count += 1;
        candidate
      };
      if node.name.is_some() && !used_names.insert(artifact_path.clone()) {
        return Err(CodecError::DuplicateArtifact(format!(
          "{FILES_PREFIX}{artifact_path}"
        )));
      }
      artifact_paths.push(artifact_path.clone());

      let start = data_offset
        .checked_add(node.data_start as usize)
        .ok_or_else(|| CodecError::Message("SARC file offset overflows".to_owned()))?;
      let end = data_offset
        .checked_add(node.data_end as usize)
        .ok_or_else(|| CodecError::Message("SARC file offset overflows".to_owned()))?;
      let bytes = data.get(start..end).ok_or_else(|| {
        CodecError::Message(format!(
          "SARC file {artifact_path:?} range {start:#x}..{end:#x} exceeds the input"
        ))
      })?;
      artifacts.push(DecodedArtifact {
        name: format!("{FILES_PREFIX}{artifact_path}"),
        data: bytes.to_vec(),
      });

      nodes.push(NodeMetadata {
        name: node.name.clone(),
        name_raw: node.name_raw.clone(),
        artifact: node.name.is_none().then(|| artifact_path.clone()),
        hash: node.hash,
        attributes: node.attributes,
        name_offset: node.name_offset,
        data_start: node.data_start,
        data_end: node.data_end,
      });
    }
    validate_file_paths(artifact_paths.iter().map(String::as_str))?;

    let name_table = data
      .get(parsed.name_table_start..parsed.name_table_start + parsed.name_table_len)
      .ok_or_else(|| CodecError::Message("SARC name table exceeds the input".to_owned()))?;
    let header_extra = &data[HEADER_SIZE..header.header_size as usize];
    let sfat_extra = &data[header.sfat_offset + SFAT_HEADER_SIZE
      ..header.sfat_offset + header.sfat_header_size as usize];
    let sfnt_extra = &data[header.sfnt_offset + SFNT_HEADER_SIZE
      ..header.sfnt_offset + header.sfnt_header_size as usize];
    let (trailing, trailing_zeros) = split_trailing(&data[header.total_size as usize..]);
    let mut ranges: Vec<(usize, usize)> = parsed
      .nodes
      .iter()
      .filter(|node| node.data_end > node.data_start)
      .map(|node| {
        (
          data_offset + node.data_start as usize,
          data_offset + node.data_end as usize,
        )
      })
      .collect();

    let metadata = SarcMetadata {
      endian: header.endian,
      header_size: header.header_size,
      version: header.version,
      reserved: header.reserved,
      data_offset: header.data_offset,
      header_extra: hex_or_empty(header_extra),
      hash_multiplier: header.hash_multiplier,
      sfat_header_size: header.sfat_header_size,
      sfat_extra: hex_or_empty(sfat_extra),
      sfnt_header_size: header.sfnt_header_size,
      sfnt_reserved: header.sfnt_reserved,
      sfnt_extra: hex_or_empty(sfnt_extra),
      name_table: hex::encode(name_table),
      data_size: header.total_size - header.data_offset,
      gaps: collect_gaps(data, data_offset, header.total_size as usize, &mut ranges)?,
      trailing,
      trailing_zeros,
      nodes,
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
    trace!("Encoding Nintendo SARC archive");
    let _args = SarcCodecArgs::from_args(args)?;

    let (metadata_artifact, file_artifacts) = artifacts.split_first().ok_or_else(|| {
      CodecError::Message(format!("Expected a {METADATA_NAME} artifact, found none"))
    })?;
    if metadata_artifact.name != METADATA_NAME {
      return Err(CodecError::Message(format!(
        "Expected the first artifact to be {METADATA_NAME}, found {}",
        metadata_artifact.name
      )));
    }
    let metadata = SarcMetadata::from_json(&metadata_artifact.data)?;
    let endian = metadata.endian;
    let node_count = u16::try_from(metadata.nodes.len()).map_err(|_| {
      CodecError::Message("SARC metadata describes more than 65535 nodes".to_owned())
    })?;
    if usize::from(metadata.header_size) < HEADER_SIZE {
      return Err(CodecError::Message(format!(
        "SARC metadata header size is {}, expected at least {HEADER_SIZE}",
        metadata.header_size
      )));
    }
    if usize::from(metadata.sfat_header_size) < SFAT_HEADER_SIZE {
      return Err(CodecError::Message(format!(
        "SARC metadata SFAT header size is {}, expected at least {SFAT_HEADER_SIZE}",
        metadata.sfat_header_size
      )));
    }
    if usize::from(metadata.sfnt_header_size) < SFNT_HEADER_SIZE {
      return Err(CodecError::Message(format!(
        "SARC metadata SFNT header size is {}, expected at least {SFNT_HEADER_SIZE}",
        metadata.sfnt_header_size
      )));
    }

    // Match artifacts to nodes and validate the names before touching layout.
    let mut used = HashSet::with_capacity(file_artifacts.len());
    let mut work = Vec::with_capacity(metadata.nodes.len());
    for node in &metadata.nodes {
      let (artifact_path, raw_name) = if let Some(name) = &node.name {
        if name.is_empty() {
          return Err(CodecError::Message(
            "SARC metadata has an empty node name".to_owned(),
          ));
        }
        validate_relative_path(name, "SARC file")?;
        (
          name.clone(),
          Some(encode_name(name, node.name_raw.as_deref())?),
        )
      } else {
        let artifact = node.artifact.as_deref().ok_or_else(|| {
          CodecError::Message(
            "SARC metadata describes an unnamed node without an artifact name".to_owned(),
          )
        })?;
        validate_relative_path(artifact, "SARC unnamed node")?;
        (artifact.to_owned(), None)
      };
      let expected = format!("{FILES_PREFIX}{artifact_path}");
      let matching: Vec<&DecodedArtifact> = file_artifacts
        .iter()
        .filter(|artifact| artifact.name == expected)
        .collect();
      let data = match matching.as_slice() {
        [] => {
          return Err(CodecError::Message(format!(
            "Expected a {expected} artifact, found none"
          )));
        }
        [artifact] => artifact.data.as_slice(),
        _ => return Err(CodecError::DuplicateArtifact(expected)),
      };
      if !used.insert(expected.clone()) {
        return Err(CodecError::DuplicateArtifact(expected));
      }
      work.push(WorkNode {
        meta: node,
        artifact_name: artifact_path,
        data,
        raw_name,
        hash: node.hash,
        attributes: node.attributes,
        name_offset: node.name_offset,
        data_start: node.data_start,
        data_end: node.data_end,
      });
    }
    for artifact in file_artifacts {
      if !used.contains(&artifact.name) {
        return Err(CodecError::Message(format!(
          "Unexpected artifact: {}",
          artifact.name
        )));
      }
    }
    validate_file_paths(work.iter().map(|node| node.artifact_name.as_str()))?;

    // Reuse the decoded name table while every name still matches; a changed
    // name means new hashes and a fresh hash-sorted node list.
    let mut name_table = decode_hex(&metadata.name_table, "name table")?;
    let mut reuse_names = true;
    for node in &work {
      match (&node.raw_name, node.name_offset) {
        (Some(raw), Some(offset))
          if node.attributes & HAS_NAME != 0 && name_matches(&name_table, offset, raw) => {}
        (None, _) if node.attributes & HAS_NAME == 0 => {}
        _ => {
          reuse_names = false;
          break;
        }
      }
    }
    if reuse_names {
      for node in &work {
        if node.meta.name.is_some() && node.attributes & HAS_NAME == 0 {
          return Err(CodecError::Message(format!(
            "SARC metadata node {:?} is named but its attributes clear the name flag",
            node.artifact_name
          )));
        }
        if node.meta.name.is_none() && node.attributes & HAS_NAME != 0 {
          return Err(CodecError::Message(format!(
            "SARC metadata node {:?} is unnamed but its attributes set the name flag",
            node.artifact_name
          )));
        }
      }
    } else {
      debug!("SARC name table rebuilt: an entry was added, removed or renamed");
      for node in &mut work {
        node.hash = match &node.raw_name {
          Some(raw) => calc_hash(raw, metadata.hash_multiplier),
          None => node.meta.hash,
        };
      }
      work.sort_by_key(|node| node.hash);
      name_table.clear();
      for node in &mut work {
        node.name_offset = match &node.raw_name {
          Some(raw) => Some(push_name(&mut name_table, raw)?),
          None => None,
        };
        node.attributes = match node.name_offset {
          Some(offset) => (node.meta.attributes & 0xFF00_0000) | HAS_NAME | (offset / 4),
          None => node.meta.attributes & !(HAS_NAME | NAME_MASK),
        };
      }
    }

    // Keep the decoded data offset while the names still fit before it; pad
    // the name table so the recorded data offset stays valid.
    let sfat_offset = metadata.header_size as usize;
    let sfnt_offset = sfat_offset
      .checked_add(metadata.sfat_header_size as usize)
      .and_then(|offset| offset.checked_add(work.len() * NODE_SIZE))
      .ok_or_else(|| CodecError::Message("SARC chunks overflow".to_owned()))?;
    let names_start = sfnt_offset
      .checked_add(metadata.sfnt_header_size as usize)
      .ok_or_else(|| CodecError::Message("SARC name table offset overflows".to_owned()))?;
    let recorded_data_offset = metadata.data_offset as usize;
    let data_offset = if names_start + name_table.len() <= recorded_data_offset {
      name_table.resize(recorded_data_offset - names_start, 0);
      recorded_data_offset
    } else {
      let offset = align_up((names_start + name_table.len()) as u64, DATA_ALIGN)
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or_else(|| CodecError::Message("SARC data offset overflows".to_owned()))?;
      // The name table runs up to the data offset; pad it, like the original.
      name_table.resize(offset - names_start, 0);
      offset
    };
    let data_offset_u32 = u32::try_from(data_offset)
      .map_err(|_| CodecError::Message("SARC data offset does not fit the header".to_owned()))?;

    // Reuse relative data offsets while sizes and the data area still agree.
    let mut reuse_layout = data_offset == recorded_data_offset;
    let mut data_size = metadata.data_size;
    if reuse_layout {
      for node in &work {
        if node.data_start > node.data_end
          || node.data_end > data_size
          || (node.data_end - node.data_start) as usize != node.data.len()
        {
          reuse_layout = false;
          break;
        }
      }
    }
    if reuse_layout {
      let mut ranges: Vec<(u64, u64)> = work
        .iter()
        .filter(|node| !node.data.is_empty())
        .map(|node| (u64::from(node.data_start), u64::from(node.data_end)))
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
          .ok_or_else(|| CodecError::Message("SARC gap range overflows".to_owned()))?;
        if gap.offset < data_offset_u32 || end > data_offset as u64 + u64::from(data_size) {
          reuse_layout = false;
          break;
        }
        // File ranges are relative to the data offset; gaps are absolute.
        gap_ranges.push((start - data_offset as u64, end - data_offset as u64));
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
    if !reuse_layout {
      debug!("SARC data layout recomputed: the decoded offsets no longer fit");
      let mut cursor = 0u64;
      for node in &mut work {
        let start = align_up(cursor, DATA_ALIGN)
          .ok_or_else(|| CodecError::Message("SARC data area overflows".to_owned()))?;
        node.data_start = u32::try_from(start).map_err(|_| {
          CodecError::Message("SARC file offset does not fit the format".to_owned())
        })?;
        cursor = start
          .checked_add(node.data.len() as u64)
          .ok_or_else(|| CodecError::Message("SARC data area overflows".to_owned()))?;
        node.data_end = u32::try_from(cursor)
          .map_err(|_| CodecError::Message("SARC file end does not fit the format".to_owned()))?;
      }
      data_size = u32::try_from(cursor)
        .map_err(|_| CodecError::Message("SARC data area does not fit the format".to_owned()))?;
    }

    let mut data = vec![0u8; data_size as usize];
    if reuse_layout {
      write_gaps(&mut data, data_offset_u32, &metadata.gaps)?;
    }
    for node in &work {
      let start = node.data_start as usize;
      let end = node.data_end as usize;
      data
        .get_mut(start..end)
        .ok_or_else(|| CodecError::Message("SARC file range exceeds the data area".to_owned()))?
        .copy_from_slice(node.data);
    }

    let total_size = data_offset
      .checked_add(data_size as usize)
      .ok_or_else(|| CodecError::Message("SARC total size overflows".to_owned()))?;
    let total_size_u32 = u32::try_from(total_size)
      .map_err(|_| CodecError::Message("SARC total size does not fit the header".to_owned()))?;
    let header_extra = extra_bytes(
      &metadata.header_extra,
      metadata.header_size as usize - HEADER_SIZE,
      "header extra",
    )?;
    let sfat_extra = extra_bytes(
      &metadata.sfat_extra,
      metadata.sfat_header_size as usize - SFAT_HEADER_SIZE,
      "SFAT extra",
    )?;
    let sfnt_extra = extra_bytes(
      &metadata.sfnt_extra,
      metadata.sfnt_header_size as usize - SFNT_HEADER_SIZE,
      "SFNT extra",
    )?;

    let mut out = Vec::with_capacity(total_size);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&endian.u16_bytes(metadata.header_size));
    out.extend_from_slice(&endian.u16_bytes(0xFEFF));
    out.extend_from_slice(&endian.u32_bytes(total_size_u32));
    out.extend_from_slice(&endian.u32_bytes(data_offset_u32));
    out.extend_from_slice(&endian.u16_bytes(metadata.version));
    out.extend_from_slice(&endian.u16_bytes(metadata.reserved));
    debug_assert_eq!(out.len(), HEADER_SIZE);
    out.extend_from_slice(&header_extra);

    out.extend_from_slice(SFAT_MAGIC);
    out.extend_from_slice(&endian.u16_bytes(metadata.sfat_header_size));
    out.extend_from_slice(&endian.u16_bytes(node_count));
    out.extend_from_slice(&endian.u32_bytes(metadata.hash_multiplier));
    out.extend_from_slice(&sfat_extra);
    for node in &work {
      out.extend_from_slice(&endian.u32_bytes(node.hash));
      out.extend_from_slice(&endian.u32_bytes(node.attributes));
      out.extend_from_slice(&endian.u32_bytes(node.data_start));
      out.extend_from_slice(&endian.u32_bytes(node.data_end));
    }

    out.extend_from_slice(SFNT_MAGIC);
    out.extend_from_slice(&endian.u16_bytes(metadata.sfnt_header_size));
    out.extend_from_slice(&endian.u16_bytes(metadata.sfnt_reserved));
    out.extend_from_slice(&sfnt_extra);
    out.extend_from_slice(&name_table);
    debug_assert_eq!(out.len(), data_offset);
    out.extend_from_slice(&data);
    out.extend_from_slice(&join_trailing(&metadata.trailing, metadata.trailing_zeros)?);
    Ok(out)
  }
}
