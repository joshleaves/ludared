//! Nintendo 3DS CIA (CTR Importable Archive) codec.
//!
//! Splits a CIA into its certificate chain, ticket, TMD, contents and meta
//! section, and rebuilds it. The TMD is parsed only far enough to locate and
//! size the contents; ticket, TMD and certificates stay opaque, since they
//! are signed blobs that cannot be regenerated without Nintendo's keys.
//!
//! Format references:
//! - <https://www.3dbrew.org/wiki/CIA>
//! - <https://www.3dbrew.org/wiki/Title_metadata>

use crate::codecs::common::bytes::{
  read_u16_be as u16_be, read_u16_le as u16_le, read_u32_be as u32_be, read_u32_le as u32_le,
  read_u64_be as u64_be, read_u64_le as u64_le,
};
use crate::codecs::common::hash::sha256;
use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::n3ds::common::{align_up, hex_field};
use crate::codecs::{Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact, LudaredCodecArgs};
use log::*;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

const CODEC_ID: &str = "std/nintendo/3ds/container/cia";
const CODEC_NAME: &str = "Nintendo 3DS CIA installable package";
const CODEC_DESC: &str = r#"Nintendo 3DS CIA (CTR Importable Archive)

Extracts a CIA package into:
- CIA.json: archive header metadata and the content table parsed from the
  TMD, including the original content index.
- certchain.bin, ticket.bin, tmd.bin: the signed sections, kept opaque.
- content0.bin, content1.bin, ...: the contents (NCCH/SRL blobs), in TMD order.
- meta.bin: the meta section, when present.
- trailer.bin: trailing bytes, when present.

Rebuilding recalculates section sizes and the 64-byte-aligned layout. By
default, the certificate chain, ticket and TMD are preserved verbatim and
content SHA-256 mismatches are rejected. For intentional same-size content
edits, set `update_content_hashes` in the codec arguments: the TMD content
hashes and their SHA-256 hash tree are refreshed, but the Nintendo RSA
signature remains unchanged and is therefore invalid for the edited TMD. This
does not produce a console-signed or stock-console-installable CIA. Content
encryption is untouched: whatever the content artifacts hold is what gets
packed.

Arguments (JSON):
- `update_content_hashes` (boolean, default false): refresh TMD content hashes
  for intentional same-size edits. The original cryptographic signature cannot
  be regenerated without Nintendo's signing key.
"#;

const HEADER_MIN_SIZE: usize = 0x2020;
const CONTENT_INDEX_OFFSET: usize = 0x20;
const CONTENT_INDEX_SIZE: usize = 0x2000;
const SECTION_ALIGN: u64 = 0x40;
const METADATA_NAME: &str = "CIA.json";
const CERTCHAIN_NAME: &str = "certchain.bin";
const TICKET_NAME: &str = "ticket.bin";
const TMD_NAME: &str = "tmd.bin";
const META_NAME: &str = "meta.bin";
const TRAILER_NAME: &str = "trailer.bin";
const TMD_CONTENT_COUNT_OFFSET: usize = 0x9E;
const TMD_CONTENT_INFO_OFFSET: usize = 0xC4;
const TMD_CONTENT_INFO_RECORD_COUNT: usize = 64;
const TMD_CONTENT_INFO_RECORD_SIZE: usize = 0x24;
const TMD_CONTENT_INFO_HASH_OFFSET: usize = 0xA4;
const TMD_CHUNK_RECORDS_OFFSET: usize = 0x9C4;
const TMD_CHUNK_RECORD_SIZE: usize = 0x30;

pub(crate) struct N3dsCia;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct N3dsCiaCodecArgs {
  #[serde(default)]
  update_content_hashes: bool,
}

impl LudaredCodecArgs for N3dsCiaCodecArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentMetadata {
  id: u32,
  index: u16,
  #[serde(rename = "type")]
  content_type: u16,
  size: u64,
  /// SHA-256 of the content, as recorded in the TMD chunk record.
  hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CiaMetadata {
  archive_header_size: u32,
  #[serde(rename = "type")]
  archive_type: u16,
  version: u16,
  /// The 0x2000-byte content index, hex-encoded.
  content_index: String,
  /// Declared content-region byte size, including content alignment padding.
  content_size: u64,

  /// Header bytes past 0x2020, when archive_header_size is larger.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  header_extra: Option<String>,
  contents: Vec<ContentMetadata>,
  /// Non-zero padding found after a section, keyed by section artifact name.
  /// Zero padding is implicit from the alignment and not recorded.
  #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
  padding: std::collections::BTreeMap<String, String>,
}

impl CiaMetadata {
  fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the CIA metadata: {err}")))
  }

  fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the CIA metadata artifact: {err}"))
    })
  }
}

fn content_name(index: usize) -> String {
  format!("content{index}.bin")
}

fn content_is_active(index_bytes: &[u8], index: u16) -> bool {
  let byte = usize::from(index) / 8;
  let bit = 7 - usize::from(index) % 8;
  index_bytes
    .get(byte)
    .is_some_and(|value| value & (1 << bit) != 0)
}

struct CiaHeader {
  archive_header_size: u32,
  archive_type: u16,
  version: u16,
  cert_size: u32,
  ticket_size: u32,
  tmd_size: u32,
  meta_size: u32,
  content_size: u64,
}

fn parse_header(data: &[u8]) -> Result<CiaHeader, CodecError> {
  let header = data.get(..HEADER_MIN_SIZE).ok_or_else(|| {
    CodecError::Message(format!(
      "CIA header is truncated: {} bytes (Minimum: {HEADER_MIN_SIZE})",
      data.len()
    ))
  })?;
  let archive_header_size = u32_le(header, 0x00)?;
  if archive_header_size < HEADER_MIN_SIZE as u32 {
    return Err(CodecError::Message(format!(
      "CIA archive header size is {archive_header_size:#x} (expected at least {HEADER_MIN_SIZE:#x})"
    )));
  }
  Ok(CiaHeader {
    archive_header_size,
    archive_type: u16_le(header, 0x04)?,
    version: u16_le(header, 0x06)?,
    cert_size: u32_le(header, 0x08)?,
    ticket_size: u32_le(header, 0x0C)?,
    tmd_size: u32_le(header, 0x10)?,
    meta_size: u32_le(header, 0x14)?,
    content_size: u64_le(header, 0x18)?,
  })
}

/// Returns the size of the TMD's signature type, signature and alignment bytes.
fn tmd_signature_data_size(tmd: &[u8]) -> Result<usize, CodecError> {
  let signature_type = u32_be(tmd, 0)?;
  // RSA-4096, RSA-2048 and ECC, in SHA-1 and SHA-256 variants.
  match signature_type {
    0x01_0000 | 0x01_0003 => Ok(0x4 + 0x200 + 0x3C),
    0x01_0001 | 0x01_0004 => Ok(0x4 + 0x100 + 0x3C),
    0x01_0002 | 0x01_0005 => Ok(0x4 + 0x3C + 0x40),
    _ => Err(CodecError::Message(format!(
      "CIA TMD signature type {signature_type:#x} is unknown"
    ))),
  }
}

/// Parses just enough of the TMD to size and order the contents.
fn parse_tmd_contents(tmd: &[u8]) -> Result<Vec<ContentMetadata>, CodecError> {
  let sig_size = tmd_signature_data_size(tmd)?;
  let count = u16_be(tmd, sig_size + TMD_CONTENT_COUNT_OFFSET)? as usize;
  let records_start = sig_size + TMD_CHUNK_RECORDS_OFFSET;
  let records_len = count
    .checked_mul(TMD_CHUNK_RECORD_SIZE)
    .ok_or_else(|| CodecError::Message("CIA TMD content record count overflows".to_owned()))?;
  let records_end = records_start
    .checked_add(records_len)
    .ok_or_else(|| CodecError::Message("CIA TMD content table overflows".to_owned()))?;
  if tmd.len() < records_end {
    return Err(CodecError::Message(format!(
      "CIA TMD is {} bytes, too short for its {count} content records (needs {records_end})",
      tmd.len()
    )));
  }

  let mut contents = Vec::with_capacity(count);
  let mut content_indices = std::collections::HashSet::with_capacity(count);
  for index in 0..count {
    let base = records_start + index * TMD_CHUNK_RECORD_SIZE;
    let content_index = u16_be(tmd, base + 0x4)?;
    if usize::from(content_index) >= CONTENT_INDEX_SIZE * 8 {
      return Err(CodecError::Message(format!(
        "CIA TMD content index {content_index} is outside the {}-bit content index",
        CONTENT_INDEX_SIZE * 8
      )));
    }
    if !content_indices.insert(content_index) {
      return Err(CodecError::Message(format!(
        "CIA TMD contains duplicate content index {content_index}"
      )));
    }
    contents.push(ContentMetadata {
      index: content_index,
      id: u32_be(tmd, base)?,
      content_type: u16_be(tmd, base + 0x6)?,
      size: u64_be(tmd, base + 0x8)?,
      hash: hex::encode(&tmd[base + 0x10..base + 0x30]),
    });
  }
  Ok(contents)
}

/// Updates the content hashes and the two TMD hash levels that cover them.
///
/// The RSA signature is deliberately not touched: no private signing key is
/// available here, and a caller opting in to content edits must not mistake a
/// refreshed SHA-256 tree for a valid Nintendo signature.
fn update_tmd_content_hashes(
  tmd: &mut [u8],
  metadata: &mut CiaMetadata,
  records: &mut [ContentMetadata],
  active_record_indices: &[usize],
  contents: &[&[u8]],
) -> Result<(), CodecError> {
  if active_record_indices.len() != contents.len() {
    return Err(CodecError::Message(
      "CIA active content records and artifacts do not match".to_owned(),
    ));
  }
  if records.len() != metadata.contents.len() {
    return Err(CodecError::Message(
      "CIA.json content records do not match the TMD".to_owned(),
    ));
  }

  let sig_size = tmd_signature_data_size(tmd)?;
  let chunk_records_start = sig_size
    .checked_add(TMD_CHUNK_RECORDS_OFFSET)
    .ok_or_else(|| CodecError::Message("CIA TMD content record offset overflows".to_owned()))?;
  let info_records_start = sig_size
    .checked_add(TMD_CONTENT_INFO_OFFSET)
    .ok_or_else(|| CodecError::Message("CIA TMD info record offset overflows".to_owned()))?;
  let info_records_size = TMD_CONTENT_INFO_RECORD_COUNT
    .checked_mul(TMD_CONTENT_INFO_RECORD_SIZE)
    .ok_or_else(|| CodecError::Message("CIA TMD info record size overflows".to_owned()))?;
  let info_records_end = info_records_start
    .checked_add(info_records_size)
    .ok_or_else(|| CodecError::Message("CIA TMD info record range overflows".to_owned()))?;
  if info_records_end != chunk_records_start || info_records_end > tmd.len() {
    return Err(CodecError::Message(
      "CIA TMD is too short for its content info records".to_owned(),
    ));
  }

  for (record_index, content) in active_record_indices.iter().zip(contents) {
    let record = records.get_mut(*record_index).ok_or_else(|| {
      CodecError::Message(format!("CIA TMD content record {record_index} is missing"))
    })?;
    if content.len() as u64 != record.size {
      return Err(CodecError::Message(format!(
        "CIA content {} is {} bytes, but the TMD records {}: content-size changes are not supported by update_content_hashes",
        record.id,
        content.len(),
        record.size
      )));
    }
    let chunk_record_start = chunk_records_start
      .checked_add(
        record_index
          .checked_mul(TMD_CHUNK_RECORD_SIZE)
          .ok_or_else(|| {
            CodecError::Message("CIA TMD content record offset overflows".to_owned())
          })?,
      )
      .ok_or_else(|| CodecError::Message("CIA TMD content record offset overflows".to_owned()))?;
    let hash_start = chunk_record_start
      .checked_add(0x10)
      .ok_or_else(|| CodecError::Message("CIA TMD content hash offset overflows".to_owned()))?;
    let hash_end = hash_start
      .checked_add(0x20)
      .ok_or_else(|| CodecError::Message("CIA TMD content hash range overflows".to_owned()))?;
    let hash = sha256(content);
    tmd
      .get_mut(hash_start..hash_end)
      .ok_or_else(|| CodecError::Message("CIA TMD content record is truncated".to_owned()))?
      .copy_from_slice(&hash);
    record.hash = hex::encode(hash);
    metadata.contents[*record_index].hash = record.hash.clone();
  }

  let mut covered_chunk_records = vec![false; records.len()];
  for index in 0..TMD_CONTENT_INFO_RECORD_COUNT {
    let info_record_start = info_records_start + index * TMD_CONTENT_INFO_RECORD_SIZE;
    let first_chunk_record = usize::from(u16_be(tmd, info_record_start)?);
    let command_count = usize::from(u16_be(tmd, info_record_start + 2)?);
    if command_count == 0 {
      continue;
    }
    let end_chunk_record = first_chunk_record
      .checked_add(command_count)
      .filter(|end| *end <= records.len())
      .ok_or_else(|| {
        CodecError::Message(
          "CIA TMD content info record range exceeds its content table".to_owned(),
        )
      })?;
    for covered in &mut covered_chunk_records[first_chunk_record..end_chunk_record] {
      if *covered {
        return Err(CodecError::Message(
          "CIA TMD content info records cover a content record more than once".to_owned(),
        ));
      }
      *covered = true;
    }
    let hashed_start = chunk_records_start
      .checked_add(
        first_chunk_record
          .checked_mul(TMD_CHUNK_RECORD_SIZE)
          .ok_or_else(|| {
            CodecError::Message("CIA TMD content record range overflows".to_owned())
          })?,
      )
      .ok_or_else(|| CodecError::Message("CIA TMD content record range overflows".to_owned()))?;
    let hashed_end = chunk_records_start
      .checked_add(
        end_chunk_record
          .checked_mul(TMD_CHUNK_RECORD_SIZE)
          .ok_or_else(|| {
            CodecError::Message("CIA TMD content record range overflows".to_owned())
          })?,
      )
      .ok_or_else(|| CodecError::Message("CIA TMD content record range overflows".to_owned()))?;
    let hash = sha256(
      tmd
        .get(hashed_start..hashed_end)
        .ok_or_else(|| CodecError::Message("CIA TMD content records are truncated".to_owned()))?,
    );
    let hash_start = info_record_start + 4;
    tmd[hash_start..hash_start + hash.len()].copy_from_slice(&hash);
  }
  let covered_count = covered_chunk_records
    .iter()
    .filter(|covered| **covered)
    .count();
  if covered_count != records.len() {
    return Err(CodecError::Message(format!(
      "CIA TMD info records cover {covered_count} of {} content records",
      records.len()
    )));
  }

  let header_hash_start = sig_size + TMD_CONTENT_INFO_HASH_OFFSET;
  let header_hash_end = header_hash_start + 0x20;
  let hash = sha256(&tmd[info_records_start..info_records_end]);
  tmd
    .get_mut(header_hash_start..header_hash_end)
    .ok_or_else(|| CodecError::Message("CIA TMD header hash field is truncated".to_owned()))?
    .copy_from_slice(&hash);

  Ok(())
}

/// Section boundaries in physical order: `(name, offset, size)`, plus the
/// content region end.
type SectionLayout = (Vec<(&'static str, u64, u64)>, u64);

/// Computes the section boundaries of an archive in physical order.
fn section_layout(header: &CiaHeader, data_len: usize) -> Result<SectionLayout, CodecError> {
  // Sections start on the next 64-byte boundary after the archive header:
  // with the usual 0x2020 header, that is 0x2040.
  let mut position = align_up(u64::from(header.archive_header_size), SECTION_ALIGN)
    .ok_or_else(|| CodecError::Message("CIA header overflows the archive".to_owned()))?;
  let data_len = u64::try_from(data_len)
    .map_err(|_| CodecError::Message("CIA archive is impossibly large".to_owned()))?;
  let mut sections = Vec::new();
  let push = |name: &'static str,
              size: u64,
              position: &mut u64,
              sections: &mut Vec<(&'static str, u64, u64)>|
   -> Result<(), CodecError> {
    if size == 0 {
      return Ok(());
    }
    let offset = *position;
    let end = offset
      .checked_add(size)
      .ok_or_else(|| CodecError::Message(format!("CIA {name} section overflows a 64-bit size")))?;
    if end > data_len {
      return Err(CodecError::Message(format!(
        "CIA {name} section ends at {end:#x}, past the end of the {data_len}-byte archive"
      )));
    }
    sections.push((name, offset, size));
    *position = align_up(end, SECTION_ALIGN)
      .ok_or_else(|| CodecError::Message(format!("CIA {name} section overflows the archive")))?;
    Ok(())
  };

  push(
    CERTCHAIN_NAME,
    u64::from(header.cert_size),
    &mut position,
    &mut sections,
  )?;
  push(
    TICKET_NAME,
    u64::from(header.ticket_size),
    &mut position,
    &mut sections,
  )?;
  push(
    TMD_NAME,
    u64::from(header.tmd_size),
    &mut position,
    &mut sections,
  )?;
  // The content region is one aligned block; its inner layout comes from the TMD.
  let content_end = position
    .checked_add(header.content_size)
    .ok_or_else(|| CodecError::Message("CIA content region overflows a 64-bit size".to_owned()))?;
  if content_end > data_len {
    return Err(CodecError::Message(format!(
      "CIA content region ends at {content_end:#x}, past the end of the {data_len}-byte archive"
    )));
  }
  position = align_up(content_end, SECTION_ALIGN)
    .ok_or_else(|| CodecError::Message("CIA content region overflows the archive".to_owned()))?;
  push(
    META_NAME,
    u64::from(header.meta_size),
    &mut position,
    &mut sections,
  )?;

  Ok((sections, content_end))
}

impl Codec for N3dsCia {
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
    let Ok(header) = parse_header(data) else {
      return Ok(CodecHandlingConfidence::No);
    };
    if header.tmd_size == 0
      || usize::try_from(header.archive_header_size)
        .ok()
        .is_none_or(|size| size > data.len())
    {
      return Ok(CodecHandlingConfidence::No);
    }
    let Ok((sections, _)) = section_layout(&header, data.len()) else {
      return Ok(CodecHandlingConfidence::No);
    };

    // The TMD must parse and its content table must fit the content region.
    let Some((_, tmd_offset, _)) = sections.iter().find(|(name, _, _)| *name == TMD_NAME) else {
      return Ok(CodecHandlingConfidence::No);
    };
    let Ok(tmd_size) = usize::try_from(header.tmd_size) else {
      return Ok(CodecHandlingConfidence::No);
    };
    let Some(tmd) = usize::try_from(*tmd_offset).ok().and_then(|start| {
      start
        .checked_add(tmd_size)
        .and_then(|end| data.get(start..end))
    }) else {
      return Ok(CodecHandlingConfidence::No);
    };
    let Ok(contents) = parse_tmd_contents(tmd) else {
      return Ok(CodecHandlingConfidence::No);
    };
    let content_index = data
      .get(CONTENT_INDEX_OFFSET..CONTENT_INDEX_OFFSET + CONTENT_INDEX_SIZE)
      .ok_or_else(|| CodecError::Message("CIA content index is truncated".to_owned()))?;
    let active_size = contents
      .iter()
      .filter(|content| content_is_active(content_index, content.index))
      .try_fold(0u64, |size, content| {
        size
          .checked_add(content.size)
          .ok_or_else(|| CodecError::Message("CIA active content sizes overflow".to_owned()))
      })
      .ok();
    let Some(content_size) = active_size else {
      return Ok(CodecHandlingConfidence::No);
    };
    if content_size > header.content_size {
      return Ok(CodecHandlingConfidence::No);
    }

    if header.cert_size > 0 && header.ticket_size > 0 && header.version == 0x20 {
      Ok(CodecHandlingConfidence::Certain)
    } else {
      Ok(CodecHandlingConfidence::Likely)
    }
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("cia".to_owned())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding Nintendo 3DS CIA package");
    let _args = N3dsCiaCodecArgs::from_args(args)?;

    let header = parse_header(data)?;
    if header.tmd_size == 0 {
      return Err(CodecError::Message("CIA package has no TMD".to_owned()));
    }
    let (sections, content_end) = section_layout(&header, data.len())?;

    let mut artifacts = Vec::new();
    let mut padding: std::collections::BTreeMap<String, String> = Default::default();

    // The content region sits between the TMD and the meta section; its start
    // bounds the padding gap that follows the TMD.
    let content_start = sections
      .iter()
      .find(|(name, _, _)| *name == TMD_NAME)
      .and_then(|(_, offset, size)| align_up(offset + size, SECTION_ALIGN))
      .ok_or_else(|| CodecError::Message("CIA package has no TMD section".to_owned()))?;

    // Sections, in physical order, recording non-zero padding.
    let mut previous: Option<(&str, u64)> = None;
    for (name, offset, size) in &sections {
      if let Some((previous_name, previous_end)) = previous {
        let gap_end = if previous_name == TMD_NAME {
          content_start
        } else {
          *offset
        };
        let gap_start = usize::try_from(previous_end).map_err(|_| {
          CodecError::Message(format!(
            "CIA padding after {previous_name} is impossibly large"
          ))
        })?;
        let gap_end = usize::try_from(gap_end).map_err(|_| {
          CodecError::Message(format!(
            "CIA padding after {previous_name} is impossibly large"
          ))
        })?;
        let gap = data.get(gap_start..gap_end).ok_or_else(|| {
          CodecError::Message(format!(
            "CIA padding after {previous_name} exceeds the archive"
          ))
        })?;
        if !gap.is_empty() && gap.iter().any(|&byte| byte != 0) {
          padding.insert(previous_name.to_owned(), hex::encode(gap));
        }
      }
      let start = usize::try_from(*offset)
        .map_err(|_| CodecError::Message(format!("CIA {name} offset is impossibly large")))?;
      let end = offset
        .checked_add(*size)
        .and_then(|end| usize::try_from(end).ok())
        .ok_or_else(|| CodecError::Message(format!("CIA {name} section range overflows")))?;
      let section = data
        .get(start..end)
        .ok_or_else(|| CodecError::Message(format!("CIA {name} section exceeds the archive")))?;
      artifacts.push(DecodedArtifact {
        name: (*name).to_owned(),
        data: section.to_vec(),
      });
      previous = Some((name, offset + size));
    }

    let tmd = artifacts
      .iter()
      .find(|artifact| artifact.name == TMD_NAME)
      .ok_or_else(|| CodecError::Message("CIA package has no TMD section".to_owned()))?;
    let all_contents = parse_tmd_contents(&tmd.data)?;
    let content_index = data
      .get(CONTENT_INDEX_OFFSET..CONTENT_INDEX_OFFSET + CONTENT_INDEX_SIZE)
      .ok_or_else(|| CodecError::Message("CIA content index is truncated".to_owned()))?;
    let contents: Vec<ContentMetadata> = all_contents
      .iter()
      .filter(|content| content_is_active(content_index, content.index))
      .cloned()
      .collect();

    if contents.is_empty() && content_end > content_start {
      let start = usize::try_from(content_start)
        .map_err(|_| CodecError::Message("CIA content start is impossibly large".to_owned()))?;
      let end = usize::try_from(content_end)
        .map_err(|_| CodecError::Message("CIA content end is impossibly large".to_owned()))?;
      let bytes = data
        .get(start..end)
        .ok_or_else(|| CodecError::Message("CIA content region exceeds the archive".to_owned()))?;
      padding.insert("content0.bin/tail".to_owned(), hex::encode(bytes));
    }

    // Split the active content region along the TMD records selected by the
    // archive-header bitmap. The bit order is MSB-first within each byte.
    let mut position = content_start;
    for (index, content) in contents.iter().enumerate() {
      if position > content_end {
        return Err(CodecError::Message(format!(
          "CIA contents end at {position:#x}, past the content region end {content_end:#x}"
        )));
      }
      let content_end_offset = position
        .checked_add(content.size)
        .filter(|&end| end <= content_end)
        .and_then(|end| usize::try_from(end).ok())
        .ok_or_else(|| {
          CodecError::Message(format!(
            "CIA content {} exceeds the declared content region ending at {content_end:#x}",
            content.id
          ))
        })?;
      let content_start_offset = usize::try_from(position).map_err(|_| {
        CodecError::Message(format!(
          "CIA content {} offset is impossibly large",
          content.id
        ))
      })?;
      let bytes = data
        .get(content_start_offset..content_end_offset)
        .ok_or_else(|| {
          CodecError::Message(format!(
            "CIA content {} is truncated ({} bytes at {position:#x})",
            content.id, content.size
          ))
        })?;
      artifacts.push(DecodedArtifact {
        name: content_name(index),
        data: bytes.to_vec(),
      });
      position = content_end_offset as u64;
      if index + 1 < contents.len() {
        let next = align_up(position, SECTION_ALIGN)
          .ok_or_else(|| CodecError::Message("CIA content region overflows".to_owned()))?;
        if next > content_end {
          return Err(CodecError::Message(format!(
            "CIA content {} alignment padding ends at {next:#x}, past the content region {content_end:#x}",
            content.id
          )));
        }
        let gap_range = usize::try_from(position)
          .ok()
          .zip(usize::try_from(next).ok())
          .ok_or_else(|| {
            CodecError::Message("CIA content padding is impossibly large".to_owned())
          })?;
        let gap = data.get(gap_range.0..gap_range.1).ok_or_else(|| {
          CodecError::Message(format!(
            "CIA content {} padding exceeds the content region",
            content.id
          ))
        })?;
        if gap.iter().any(|&byte| byte != 0) {
          padding.insert(content_name(index), hex::encode(gap));
        }
        position = next;
      }
    }
    if position > content_end {
      return Err(CodecError::Message(format!(
        "CIA contents end at {position:#x}, past the content region end {content_end:#x}"
      )));
    }
    let tail_start = usize::try_from(position)
      .map_err(|_| CodecError::Message("CIA content tail is impossibly large".to_owned()))?;
    let tail_end = usize::try_from(content_end)
      .map_err(|_| CodecError::Message("CIA content end is impossibly large".to_owned()))?;
    let tail = data
      .get(tail_start..tail_end)
      .ok_or_else(|| CodecError::Message("CIA content tail exceeds the archive".to_owned()))?;
    if contents.is_empty() {
      if !tail.is_empty() {
        padding.insert("content0.bin/tail".to_owned(), hex::encode(tail));
      }
    } else if tail.iter().any(|&byte| byte != 0) {
      padding.insert(
        content_name(contents.len() - 1) + "/tail",
        hex::encode(tail),
      );
    }

    // Meta and trailer.
    let meta_end = align_up(content_end, SECTION_ALIGN)
      .and_then(|aligned| aligned.checked_add(u64::from(header.meta_size)))
      .ok_or_else(|| CodecError::Message("CIA meta section end overflows".to_owned()))?;
    let meta_end = usize::try_from(meta_end)
      .map_err(|_| CodecError::Message("CIA meta section end is impossibly large".to_owned()))?;
    if let Some(trailer) = data.get(meta_end..).filter(|trailer| !trailer.is_empty()) {
      artifacts.push(DecodedArtifact {
        name: TRAILER_NAME.to_owned(),
        data: trailer.to_vec(),
      });
    }

    let archive_header_size = usize::try_from(header.archive_header_size)
      .map_err(|_| CodecError::Message("CIA archive header size is impossibly large".to_owned()))?;
    let header_extra = if archive_header_size > HEADER_MIN_SIZE {
      let extra = data
        .get(HEADER_MIN_SIZE..archive_header_size)
        .ok_or_else(|| {
          CodecError::Message("CIA extended archive header exceeds the archive".to_owned())
        })?;
      Some(hex::encode(extra))
    } else {
      None
    };

    let metadata = CiaMetadata {
      archive_header_size: header.archive_header_size,
      archive_type: header.archive_type,
      version: header.version,
      content_index: hex::encode(
        &data[CONTENT_INDEX_OFFSET..CONTENT_INDEX_OFFSET + CONTENT_INDEX_SIZE],
      ),
      header_extra,
      content_size: header.content_size,
      contents: all_contents,
      padding,
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
    trace!("Encoding Nintendo 3DS CIA package");
    let codec_args = N3dsCiaCodecArgs::from_args(args)?;

    let (metadata_artifact, rest) = artifacts.split_first().ok_or_else(|| {
      CodecError::Message(format!("Expected a {METADATA_NAME} artifact, found none"))
    })?;
    if metadata_artifact.name != METADATA_NAME {
      return Err(CodecError::Message(format!(
        "Expected the first artifact to be {METADATA_NAME}, found {}",
        metadata_artifact.name
      )));
    }
    let mut metadata = CiaMetadata::from_json(&metadata_artifact.data)?;

    let take = |name: &str, required: bool| -> Result<Option<&[u8]>, CodecError> {
      let matching: Vec<&[u8]> = rest
        .iter()
        .filter(|artifact| artifact.name == name)
        .map(|artifact| artifact.data.as_slice())
        .collect();
      match (matching.as_slice(), required) {
        ([bytes], _) => Ok(Some(bytes)),
        ([], false) => Ok(None),
        ([], true) => Err(CodecError::Message(format!(
          "Expected a {name} artifact, found none"
        ))),
        _ => Err(CodecError::DuplicateArtifact(name.to_owned())),
      }
    };

    let certchain = take(CERTCHAIN_NAME, true)?.ok_or_else(|| {
      CodecError::Message(format!("Expected a {CERTCHAIN_NAME} artifact, found none"))
    })?;
    let ticket = take(TICKET_NAME, true)?.ok_or_else(|| {
      CodecError::Message(format!("Expected a {TICKET_NAME} artifact, found none"))
    })?;
    let mut tmd = take(TMD_NAME, true)?
      .ok_or_else(|| CodecError::Message(format!("Expected a {TMD_NAME} artifact, found none")))?
      .to_vec();
    let meta = take(META_NAME, false)?;
    let trailer = take(TRAILER_NAME, false)?;

    // The TMD is the signed source of truth for the content table. Intentional
    // hash updates may change only its derived SHA-256 fields, never record
    // identity, size, ordering, or content-index semantics.
    let mut records = parse_tmd_contents(&tmd)?;
    if codec_args.update_content_hashes {
      let same_structure = records.len() == metadata.contents.len()
        && records
          .iter()
          .zip(&metadata.contents)
          .all(|(record, metadata_record)| {
            record.id == metadata_record.id
              && record.index == metadata_record.index
              && record.content_type == metadata_record.content_type
              && record.size == metadata_record.size
          });
      if !same_structure {
        return Err(CodecError::Message(
          "CIA.json does not match tmd.bin: content record identity or size differs".to_owned(),
        ));
      }
    } else if records != metadata.contents {
      return Err(CodecError::Message(
        "CIA.json does not match tmd.bin: the content table in the metadata is not the one the TMD signs. \
         After editing contents, regenerate the TMD externally and refresh CIA.json from it".to_owned(),
      ));
    }

    let content_index = hex_field(&metadata.content_index, "content index", CONTENT_INDEX_SIZE)?;
    let active_record_indices: Vec<usize> = records
      .iter()
      .enumerate()
      .filter(|(_, record)| content_is_active(&content_index, record.index))
      .map(|(index, _)| index)
      .collect();
    if active_record_indices.len()
      != metadata
        .contents
        .iter()
        .filter(|record| content_is_active(&content_index, record.index))
        .count()
    {
      return Err(CodecError::Message(
        "CIA content metadata and active content index disagree".to_owned(),
      ));
    }
    let mut contents = Vec::with_capacity(active_record_indices.len());
    for (index, record_index) in active_record_indices.iter().enumerate() {
      let record = &records[*record_index];
      let name = content_name(index);
      let bytes = take(&name, true)?
        .ok_or_else(|| CodecError::Message(format!("Expected a {name} artifact, found none")))?;
      if bytes.len() as u64 != record.size {
        return Err(CodecError::Message(format!(
          "CIA content {} is {:#x} bytes, but the TMD records {:#x}: \
           the TMD must be regenerated for resized contents",
          record.id,
          bytes.len(),
          record.size
        )));
      }
      if !codec_args.update_content_hashes {
        let expected_hash = hex_field(&record.hash, "TMD content hash", 0x20)?;
        let actual_hash = sha256(bytes);
        if actual_hash.as_slice() != expected_hash {
          return Err(CodecError::Message(format!(
            "CIA content {} does not match the SHA-256 recorded in its TMD",
            record.id
          )));
        }
      }
      contents.push(bytes);
    }
    if codec_args.update_content_hashes {
      update_tmd_content_hashes(
        &mut tmd,
        &mut metadata,
        &mut records,
        &active_record_indices,
        &contents,
      )?;
    }

    let known: Vec<String> = [
      CERTCHAIN_NAME,
      TICKET_NAME,
      TMD_NAME,
      META_NAME,
      TRAILER_NAME,
    ]
    .into_iter()
    .map(str::to_owned)
    .chain((0..active_record_indices.len()).map(content_name))
    .collect();
    for artifact in rest {
      if !known.contains(&artifact.name) {
        return Err(CodecError::Message(format!(
          "Unexpected artifact: {}",
          artifact.name
        )));
      }
    }

    let header_extra = metadata
      .header_extra
      .as_deref()
      .map(hex::decode)
      .transpose()
      .map_err(|err| {
        CodecError::Message(format!(
          "CIA metadata header extra is not valid hexadecimal: {err}"
        ))
      })?;
    let archive_header_size = HEADER_MIN_SIZE + header_extra.as_ref().map_or(0, Vec::len);
    if archive_header_size != metadata.archive_header_size as usize {
      return Err(CodecError::Message(format!(
        "CIA metadata header extra is {} bytes, but archive_header_size says {:#x}",
        header_extra.as_ref().map_or(0, Vec::len),
        metadata.archive_header_size
      )));
    }

    let padding_for = |name: &str| -> Result<Vec<u8>, CodecError> {
      metadata
        .padding
        .get(name)
        .map(hex::decode)
        .transpose()
        .map_err(|err| {
          CodecError::Message(format!(
            "CIA metadata padding of {name} is not valid hexadecimal: {err}"
          ))
        })
        .map(|padding| padding.unwrap_or_default())
    };

    let mut image = Vec::new();
    let mut header = vec![0u8; archive_header_size];
    header[CONTENT_INDEX_OFFSET..CONTENT_INDEX_OFFSET + CONTENT_INDEX_SIZE]
      .copy_from_slice(&content_index);
    if let Some(extra) = &header_extra {
      header[HEADER_MIN_SIZE..].copy_from_slice(extra);
    }

    let put = |image: &mut Vec<u8>, name: &str, bytes: &[u8]| -> Result<(), CodecError> {
      let padding = padding_for(name)?;
      image.extend_from_slice(bytes);
      let expected_padding = align_up(image.len() as u64, SECTION_ALIGN)
        .and_then(|aligned| usize::try_from(aligned - image.len() as u64).ok())
        .ok_or_else(|| CodecError::Message("CIA layout overflows".to_owned()))?;
      if !padding.is_empty() {
        if padding.len() != expected_padding {
          return Err(CodecError::Message(format!(
            "CIA padding after {name} is {} bytes, expected {expected_padding}",
            padding.len()
          )));
        }
        image.extend_from_slice(&padding);
      } else {
        image.resize(image.len() + expected_padding, 0);
      }
      Ok(())
    };

    image.extend_from_slice(&header);
    let first_section = align_up(image.len() as u64, SECTION_ALIGN)
      .ok_or_else(|| CodecError::Message("CIA layout overflows".to_owned()))?;
    image.extend(std::iter::repeat_n(
      0,
      (first_section - image.len() as u64) as usize,
    ));
    put(&mut image, CERTCHAIN_NAME, certchain)?;
    put(&mut image, TICKET_NAME, ticket)?;
    put(&mut image, TMD_NAME, &tmd)?;

    let content_start = image.len() as u64;
    for (index, bytes) in contents.iter().enumerate() {
      image.extend_from_slice(bytes);
      if index + 1 < contents.len() {
        let gap = padding_for(&content_name(index))?;
        let expected_size = align_up(image.len() as u64, SECTION_ALIGN)
          .and_then(|next| usize::try_from(next - image.len() as u64).ok())
          .ok_or_else(|| CodecError::Message("CIA content alignment overflows".to_owned()))?;
        if !gap.is_empty() {
          if gap.len() != expected_size {
            return Err(CodecError::Message(format!(
              "CIA content {} alignment padding is {} bytes, expected {expected_size}",
              index,
              gap.len()
            )));
          }
          image.extend_from_slice(&gap);
        } else {
          image.resize(image.len() + expected_size, 0);
        }
      }
    }
    let current_content_size = image.len() as u64 - content_start;
    if current_content_size > metadata.content_size {
      return Err(CodecError::Message(format!(
        "CIA content artifacts and alignment take {current_content_size} bytes, exceeding the recorded content region of {} bytes",
        metadata.content_size
      )));
    }
    let tail_key = if contents.is_empty() {
      "content0.bin/tail".to_owned()
    } else {
      content_name(contents.len() - 1) + "/tail"
    };
    let tail = padding_for(&tail_key)?;
    let content_tail_len = usize::try_from(metadata.content_size - current_content_size)
      .map_err(|_| CodecError::Message("CIA content tail is impossibly large".to_owned()))?;
    if tail.len() > content_tail_len {
      return Err(CodecError::Message(format!(
        "CIA content tail is {} bytes, exceeding its {content_tail_len}-byte region",
        tail.len()
      )));
    }
    image.extend_from_slice(&tail);
    image.resize(image.len() + content_tail_len - tail.len(), 0);
    let content_size = metadata.content_size;
    if image.len() as u64 != content_start + content_size {
      return Err(CodecError::Message(
        "CIA content region did not reach its declared end".to_owned(),
      ));
    }
    let aligned = align_up(image.len() as u64, SECTION_ALIGN)
      .ok_or_else(|| CodecError::Message("CIA layout overflows".to_owned()))?;
    image.extend(std::iter::repeat_n(
      0,
      (aligned - image.len() as u64) as usize,
    ));

    if let Some(meta) = meta {
      image.extend_from_slice(meta);
    }
    if let Some(trailer) = trailer {
      image.extend_from_slice(trailer);
    }

    // Patch the size fields into the header.
    let mut write = |offset: usize, bytes: &[u8]| {
      image[offset..offset + bytes.len()].copy_from_slice(bytes);
    };
    write(0x00, &(metadata.archive_header_size).to_le_bytes());
    write(0x04, &metadata.archive_type.to_le_bytes());
    write(0x06, &metadata.version.to_le_bytes());
    write(
      0x08,
      &u32::try_from(certchain.len())
        .map_err(|_| CodecError::Message("CIA certificate chain is too large".to_owned()))?
        .to_le_bytes(),
    );
    write(
      0x0C,
      &u32::try_from(ticket.len())
        .map_err(|_| CodecError::Message("CIA ticket is too large".to_owned()))?
        .to_le_bytes(),
    );
    write(
      0x10,
      &u32::try_from(tmd.len())
        .map_err(|_| CodecError::Message("CIA TMD is too large".to_owned()))?
        .to_le_bytes(),
    );
    write(
      0x14,
      &u32::try_from(meta.map_or(0, <[u8]>::len))
        .map_err(|_| CodecError::Message("CIA meta section is too large".to_owned()))?
        .to_le_bytes(),
    );
    write(0x18, &content_size.to_le_bytes());

    Ok(image)
  }
}
