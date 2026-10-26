//! Nintendo 3DS CCI (NCSD) cartridge image codec.
//!
//! Extracts header metadata, opaque card info, NCCH partitions and trailing
//! bytes. Rebuilding recalculates partition offsets and sizes, but does not
//! decrypt partitions or verify or regenerate signatures.
//!
//! The declared image size is a capacity, not necessarily the physical length:
//! trimmed images are accepted, but a rebuild cannot exceed that capacity.

use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::n3ds::common::{field, u32_le, u64_le};
use crate::codecs::nintendo::n3ds::container::common::media_unit_size;
use log::*;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(crate) mod metadata;

#[cfg(test)]
mod tests;

use metadata::{
  CARD_INFO_NAME, METADATA_NAME, NcsdMetadata, PartitionMetadata, TRAILER_NAME, partition_index,
  partition_name,
};

const CODEC_ID: &str = "std/nintendo/3ds/container/cci";
const CODEC_NAME: &str = "Nintendo 3DS CCI (NCSD) cartridge image";
const CODEC_DESC: &str = r#"Nintendo 3DS CCI (CTR Cart Image)

Extracts NCSD cartridge images into:
- NCSD.json: cartridge header metadata, including the original RSA signature.
- cardinfo.bin: opaque data between the header and the first partition.
- partitionN.ncch: complete NCCH partitions, preserved without decryption.
- trailer.bin: trailing data, when present.

Rebuilding recalculates partition offsets and sizes while preserving the
remaining header fields and opaque data. Partitions can be resized or replaced,
provided they remain media-unit aligned and fit within the declared image
capacity (image_size in NCSD.json).

This codec does not decrypt NCCH partitions, validate signatures or generate
new ones. Modified images are not guaranteed to boot on retail hardware.

No arguments.
"#;

const HEADER_SIZE: usize = 0x200;
const MAGIC_OFFSET: usize = 0x100;
const NCSD_MAGIC: &[u8; 4] = b"NCSD";
const NCCH_MAGIC: &[u8; 4] = b"NCCH";
const NCCH_MAGIC_OFFSET: u64 = 0x100;
const PARTITION_COUNT: usize = 8;
const PARTITION_TABLE_OFFSET: usize = 0x120;
const PARTITION_ID_TABLE_OFFSET: usize = 0x190;
const EXHEADER_HASH_OFFSET: usize = 0x160;
const EXHEADER_HASH_SIZE: usize = 0x20;
const RESERVED_OFFSET: usize = 0x1D0;
const RESERVED_SIZE: usize = 0x30;
const FIRST_PARTITION_OFFSET: u64 = 0x4000;
const FLAG_MEDIA_PLATFORM: usize = 4;
const FLAG_MEDIA_TYPE: usize = 5;
const FLAG_MEDIA_UNIT: usize = 6;
const MEDIA_PLATFORM_CTR: u8 = 1;
const MEDIA_TYPE_CARD1: u8 = 1;
const MEDIA_TYPE_CARD2: u8 = 2;

// Retail capacities, in media units. Other values are unusual, not invalid.
const KNOWN_IMAGE_SIZES: [u32; 7] = [
  0x40000,
  0x80000,
  0x100000,
  0x200000,
  0x400000,
  0x800000,
  0x1000000,
];

pub(crate) struct N3dsCci;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct N3dsCciCodecArgs {}

impl LudaredCodecArgs for N3dsCciCodecArgs {}

impl Codec for N3dsCci {
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
    let Ok(header) = NcsdHeader::parse(data) else {
      return Ok(CodecHandlingConfidence::No);
    };

    // CTR NAND also uses NCSD, but is not a cartridge.
    if header.flags[FLAG_MEDIA_PLATFORM] != MEDIA_PLATFORM_CTR
      || !matches!(
        header.flags[FLAG_MEDIA_TYPE],
        MEDIA_TYPE_CARD1 | MEDIA_TYPE_CARD2
      )
      || header.media_id == 0
    {
      return Ok(CodecHandlingConfidence::No);
    }

    let Ok(layout) = header.layout(data.len()) else {
      return Ok(CodecHandlingConfidence::No);
    };

    let known_size = KNOWN_IMAGE_SIZES.contains(&header.image_size);
    let ncch = layout
      .partitions
      .iter()
      .all(|partition| carries_ncch_magic(data, partition));

    Ok(match (known_size, ncch) {
      (true, true) => CodecHandlingConfidence::Certain,
      (true, false) | (false, true) => CodecHandlingConfidence::Likely,
      (false, false) => CodecHandlingConfidence::No,
    })
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("cci".to_owned())
  }

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding Nintendo 3DS CCI image");
    let _args = N3dsCciCodecArgs::from_args(args)?;

    let header = NcsdHeader::parse(data)?;
    let layout = header.layout(data.len())?;
    let metadata = header.metadata(&layout);

    let mut artifacts = Vec::with_capacity(layout.partitions.len() + 3);
    artifacts.push(DecodedArtifact {
      name: METADATA_NAME.to_owned(),
      data: metadata.to_json()?,
    });

    let first_offset = usize::try_from(layout.first_offset)
      .map_err(|_| CodecError::Message("NCSD card info region is impossibly large".to_owned()))?;
    let card_info = data
      .get(HEADER_SIZE..first_offset)
      .ok_or_else(|| CodecError::Message("NCSD card info region is truncated".to_owned()))?;
    artifacts.push(DecodedArtifact {
      name: CARD_INFO_NAME.to_owned(),
      data: card_info.to_vec(),
    });

    for partition in &layout.partitions {
      let offset = usize::try_from(partition.offset)
        .map_err(|_| CodecError::Message("NCSD partition offset is impossibly large".to_owned()))?;
      let size = usize::try_from(partition.size)
        .map_err(|_| CodecError::Message("NCSD partition size is impossibly large".to_owned()))?;
      let bytes = data
        .get(offset..)
        .and_then(|rest| rest.get(..size))
        .ok_or_else(|| {
          CodecError::Message(format!("NCSD partition {} is truncated", partition.index))
        })?;

      artifacts.push(DecodedArtifact {
        name: partition_name(partition.index),
        data: bytes.to_vec(),
      });
    }

    let used_end = usize::try_from(layout.used_end)
      .map_err(|_| CodecError::Message("NCSD partition layout is impossibly large".to_owned()))?;
    if let Some(trailer) = data.get(used_end..).filter(|trailer| !trailer.is_empty()) {
      artifacts.push(DecodedArtifact {
        name: TRAILER_NAME.to_owned(),
        data: trailer.to_vec(),
      });
    }

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
    trace!("Encoding Nintendo 3DS CCI image");
    let _args = N3dsCciCodecArgs::from_args(args)?;

    let SortedArtifacts {
      metadata,
      card_info,
      trailer,
      partitions,
    } = N3dsCci::sort_artifacts(artifacts)?;
    let metadata = NcsdMetadata::from_json(metadata)?;
    let entries = N3dsCci::ordered_partitions(&metadata, &partitions)?;

    let media_unit = media_unit_size(metadata.flags[FLAG_MEDIA_UNIT]).ok_or_else(|| {
      CodecError::Message(format!(
        "NCSD metadata media unit size flag {} overflows a 64-bit size",
        metadata.flags[FLAG_MEDIA_UNIT]
      ))
    })?;

    let card_info_len = u64::try_from(card_info.len())
      .map_err(|_| CodecError::Message("NCSD card info region is impossibly large".to_owned()))?;
    let first_offset = (HEADER_SIZE as u64)
      .checked_add(card_info_len)
      .ok_or_else(|| CodecError::Message("NCSD card info region overflows the image".to_owned()))?;

    if first_offset < FIRST_PARTITION_OFFSET {
      return Err(CodecError::Message(format!(
        "NCSD card info region ends at {first_offset:#x}, leaving no room for the header; \
        the first partition must start at {FIRST_PARTITION_OFFSET:#x}"
      )));
    }
    if !first_offset.is_multiple_of(media_unit) {
      return Err(CodecError::Message(format!(
        "NCSD card info region of {card_info_len} bytes does not end on a {media_unit}-byte media unit boundary"
      )));
    }

    // The final position is already known while building the table.
    let (table, used_end) = N3dsCci::partition_table(&entries, first_offset, media_unit)?;

    let capacity = u64::from(metadata.image_size)
      .checked_mul(media_unit)
      .ok_or_else(|| {
        CodecError::Message(format!(
          "NCSD image size {} media units overflows a 64-bit size",
          metadata.image_size
        ))
      })?;
    if used_end > capacity {
      return Err(CodecError::Message(format!(
        "NCSD partitions would end at {used_end:#x}, past the declared image size of {capacity:#x}"
      )));
    }

    // Keep trailing bytes verbatim; never silently extend the declared capacity.
    let trailer_len = u64::try_from(trailer.map_or(0usize, <[u8]>::len))
      .map_err(|_| CodecError::Message("NCSD trailing region is impossibly large".to_owned()))?;
    let image_len = used_end
      .checked_add(trailer_len)
      .ok_or_else(|| CodecError::Message("NCSD image length overflows a 64-bit size".to_owned()))?;
    if image_len > capacity {
      return Err(CodecError::Message(format!(
        "NCSD image would be {image_len:#x} bytes, past the declared image size of {capacity:#x}: \
         the {trailer_len:#x}-byte trailing region no longer fits. Raise image_size in \
         {METADATA_NAME} to rebuild this layout"
      )));
    }

    let header = N3dsCci::header_bytes(&metadata, &table)?;

    let mut image = Vec::new();
    image.extend_from_slice(&header);
    image.extend_from_slice(card_info);
    for &(_, bytes) in &entries {
      image.extend_from_slice(bytes);
    }
    if let Some(trailer) = trailer {
      image.extend_from_slice(trailer);
    }

    Ok(image)
  }
}

struct PresentPartition {
  index: usize,
  offset: u64,
  offset_units: u32,
  size: u64,
  size_units: u32,
}

struct Layout {
  first_offset: u64,
  used_end: u64,
  partitions: Vec<PresentPartition>,
}

struct NcsdHeader {
  signature: [u8; 0x100],
  image_size: u32,
  media_id: u64,
  partition_fs_types: [u8; 8],
  partition_crypto_types: [u8; 8],
  partition_table: [(u32, u32); PARTITION_COUNT],
  exheader_hash: [u8; EXHEADER_HASH_SIZE],
  additional_header_size: u32,
  sector_zero_offset: u32,
  flags: [u8; 8],
  partition_ids: [u64; PARTITION_COUNT],
  header_reserved: [u8; RESERVED_SIZE],
}

impl NcsdHeader {
  fn parse(data: &[u8]) -> Result<Self, CodecError> {
    let header = data.get(..HEADER_SIZE).ok_or_else(|| {
      CodecError::Message(format!(
        "NCSD header is truncated: {} bytes (Minimum: {HEADER_SIZE})",
        data.len()
      ))
    })?;

    if header.get(MAGIC_OFFSET..MAGIC_OFFSET + NCSD_MAGIC.len()) != Some(NCSD_MAGIC.as_slice()) {
      return Err(CodecError::Message(
        "Not an NCSD image: the 'NCSD' magic is missing at 0x100".to_owned(),
      ));
    }

    let mut partition_table = [(0u32, 0u32); PARTITION_COUNT];
    for (index, entry) in partition_table.iter_mut().enumerate() {
      let base = PARTITION_TABLE_OFFSET + index * 8;
      entry.0 = u32_le(header, base)?;
      entry.1 = u32_le(header, base + 4)?;
    }

    let mut partition_ids = [0u64; PARTITION_COUNT];
    for (index, id) in partition_ids.iter_mut().enumerate() {
      *id = u64_le(header, PARTITION_ID_TABLE_OFFSET + index * 8)?;
    }

    Ok(Self {
      signature: field(header, 0)?,
      image_size: u32_le(header, 0x104)?,
      media_id: u64_le(header, 0x108)?,
      partition_fs_types: field(header, 0x110)?,
      partition_crypto_types: field(header, 0x118)?,
      partition_table,
      exheader_hash: field(header, EXHEADER_HASH_OFFSET)?,
      additional_header_size: u32_le(header, 0x180)?,
      sector_zero_offset: u32_le(header, 0x184)?,
      flags: field(header, 0x188)?,
      partition_ids,
      header_reserved: field(header, RESERVED_OFFSET)?,
    })
  }

  fn media_unit(&self) -> Result<u64, CodecError> {
    media_unit_size(self.flags[FLAG_MEDIA_UNIT]).ok_or_else(|| {
      CodecError::Message(format!(
        "NCSD media unit size flag {} overflows a 64-bit size",
        self.flags[FLAG_MEDIA_UNIT]
      ))
    })
  }

  fn layout(&self, image_len: usize) -> Result<Layout, CodecError> {
    let media_unit = self.media_unit()?;
    let mut partitions = Vec::new();

    for (index, (offset_units, size_units)) in self.partition_table.iter().enumerate() {
      match (*offset_units, *size_units) {
        (0, 0) => continue,
        (0, _) => {
          return Err(CodecError::Message(format!(
            "NCSD partition {index} has a size but a zero offset"
          )));
        }
        (_, 0) => {
          return Err(CodecError::Message(format!(
            "NCSD partition {index} has an offset but a zero size"
          )));
        }
        _ => {}
      }

      let offset = u64::from(*offset_units)
        .checked_mul(media_unit)
        .ok_or_else(|| partition_overflow("offset", index))?;
      let size = u64::from(*size_units)
        .checked_mul(media_unit)
        .ok_or_else(|| partition_overflow("size", index))?;

      partitions.push(PresentPartition {
        index,
        offset,
        offset_units: *offset_units,
        size,
        size_units: *size_units,
      });
    }

    let Some(first) = partitions.first() else {
      return Err(CodecError::Message(
        "NCSD image declares no partitions".to_owned(),
      ));
    };

    if first.index != 0 {
      return Err(CodecError::Message(format!(
        "NCSD partition 0 is missing; the first partition present is {}",
        first.index
      )));
    }

    if first.offset < FIRST_PARTITION_OFFSET {
      return Err(CodecError::Message(format!(
        "NCSD partition 0 starts at {:#x}, inside the card info region (first valid: {FIRST_PARTITION_OFFSET:#x})",
        first.offset
      )));
    }

    let mut expected = first.offset;
    for partition in &partitions {
      if partition.offset != expected {
        return Err(CodecError::Message(format!(
          "NCSD partition {} starts at {:#x}, but the previous partition ends at {expected:#x}: the partitions are not contiguous",
          partition.index, partition.offset
        )));
      }

      expected = partition
        .offset
        .checked_add(partition.size)
        .ok_or_else(|| partition_overflow("end", partition.index))?;
    }

    let used_end = expected;
    let capacity = u64::from(self.image_size)
      .checked_mul(media_unit)
      .ok_or_else(|| {
        CodecError::Message(format!(
          "NCSD image size {} media units overflows a 64-bit size",
          self.image_size
        ))
      })?;

    if used_end > capacity {
      return Err(CodecError::Message(format!(
        "NCSD partitions end at {used_end:#x}, past the declared image size of {capacity:#x}"
      )));
    }
    if used_end > image_len as u64 {
      return Err(CodecError::Message(format!(
        "NCSD partitions end at {used_end:#x}, past the end of the {image_len}-byte image"
      )));
    }
    if image_len as u64 > capacity {
      return Err(CodecError::Message(format!(
        "NCSD image is {image_len} bytes, past its declared image size of {capacity:#x}"
      )));
    }

    Ok(Layout {
      first_offset: first.offset,
      used_end,
      partitions,
    })
  }

  fn metadata(&self, layout: &Layout) -> NcsdMetadata {
    NcsdMetadata {
      signature: hex::encode(self.signature),
      image_size: self.image_size,
      media_id: self.media_id,
      partition_fs_types: self.partition_fs_types,
      partition_crypto_types: self.partition_crypto_types,
      flags: self.flags,
      exheader_hash: hex::encode(self.exheader_hash),
      additional_header_size: self.additional_header_size,
      sector_zero_offset: self.sector_zero_offset,
      partition_ids: self.partition_ids,
      header_reserved: hex::encode(self.header_reserved),
      partitions: layout
        .partitions
        .iter()
        .map(|partition| PartitionMetadata {
          index: partition.index,
          offset: partition.offset_units,
          size: partition.size_units,
        })
        .collect(),
    }
  }
}

struct SortedArtifacts<'a> {
  metadata: &'a [u8],
  card_info: &'a [u8],
  trailer: Option<&'a [u8]>,
  partitions: BTreeMap<usize, &'a [u8]>,
}

impl N3dsCci {
  fn sort_artifacts(artifacts: &[DecodedArtifact]) -> Result<SortedArtifacts<'_>, CodecError> {
    let mut metadata = None;
    let mut card_info = None;
    let mut trailer = None;
    let mut partitions: BTreeMap<usize, &[u8]> = BTreeMap::new();

    for artifact in artifacts {
      let bytes = artifact.data.as_slice();
      match artifact.name.as_str() {
        METADATA_NAME => {
          if metadata.replace(bytes).is_some() {
            return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
          }
        }
        CARD_INFO_NAME => {
          if card_info.replace(bytes).is_some() {
            return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
          }
        }
        TRAILER_NAME => {
          if trailer.replace(bytes).is_some() {
            return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
          }
        }
        name => match partition_index(name) {
          Some(index) if index < PARTITION_COUNT => {
            if partitions.insert(index, bytes).is_some() {
              return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
            }
          }
          _ => return Err(CodecError::Message(format!("Unexpected artifact: {name}"))),
        },
      }
    }

    let metadata = metadata.ok_or_else(|| {
      CodecError::Message(format!("Expected a {METADATA_NAME} artifact, found none"))
    })?;
    let card_info = card_info.ok_or_else(|| {
      CodecError::Message(format!("Expected a {CARD_INFO_NAME} artifact, found none"))
    })?;

    Ok(SortedArtifacts {
      metadata,
      card_info,
      trailer,
      partitions,
    })
  }

  fn ordered_partitions<'a>(
    metadata: &'a NcsdMetadata,
    partitions: &'a BTreeMap<usize, &'a [u8]>,
  ) -> Result<Vec<(&'a PartitionMetadata, &'a [u8])>, CodecError> {
    let mut entries: Vec<&PartitionMetadata> = metadata.partitions.iter().collect();
    entries.sort_by_key(|entry| entry.index);

    for (position, entry) in entries.iter().enumerate() {
      if entry.index >= PARTITION_COUNT {
        return Err(CodecError::Message(format!(
          "NCSD metadata describes partition {}, but there are only {PARTITION_COUNT}",
          entry.index
        )));
      }

      if position > 0 && entries[position - 1].index == entry.index {
        return Err(CodecError::Message(format!(
          "NCSD metadata describes partition {} twice",
          entry.index
        )));
      }
    }

    if entries.first().map(|entry| entry.index) != Some(0) {
      return Err(CodecError::Message(
        "NCSD metadata does not describe partition 0".to_owned(),
      ));
    }

    for index in partitions.keys() {
      if !entries.iter().any(|entry| entry.index == *index) {
        return Err(CodecError::Message(format!(
          "Unexpected artifact: {}",
          partition_name(*index)
        )));
      }
    }

    entries
      .into_iter()
      .map(|entry| {
        let artifact = partitions.get(&entry.index).ok_or_else(|| {
          CodecError::Message(format!(
            "Expected a {} artifact, found none",
            partition_name(entry.index)
          ))
        })?;

        Ok((entry, *artifact))
      })
      .collect()
  }

  // Return the end offset alongside the table, avoiding a second traversal.
  fn partition_table(
    entries: &[(&PartitionMetadata, &[u8])],
    first_offset: u64,
    media_unit: u64,
  ) -> Result<([(u32, u32); PARTITION_COUNT], u64), CodecError> {
    let mut table = [(0u32, 0u32); PARTITION_COUNT];
    let mut running = first_offset;

    for &(entry, bytes) in entries {
      let size = u64::try_from(bytes.len()).map_err(|_| {
        CodecError::Message(format!(
          "NCSD partition {} is impossibly large",
          entry.index
        ))
      })?;

      if !running.is_multiple_of(media_unit) {
        return Err(CodecError::Message(format!(
          "NCSD partition {} starts at {running:#x}, off a {media_unit}-byte media unit boundary",
          entry.index
        )));
      }
      if !size.is_multiple_of(media_unit) {
        return Err(CodecError::Message(format!(
          "NCSD partition {} is {size} bytes, not a whole number of {media_unit}-byte media units",
          entry.index
        )));
      }

      let offset_units = u32::try_from(running / media_unit).map_err(|_| {
        CodecError::Message(format!(
          "NCSD partition {} offset does not fit the partition table",
          entry.index
        ))
      })?;
      let size_units = u32::try_from(size / media_unit).map_err(|_| {
        CodecError::Message(format!(
          "NCSD partition {} size does not fit the partition table",
          entry.index
        ))
      })?;

      // ordered_partitions has already validated every index.
      table[entry.index] = (offset_units, size_units);

      running = running.checked_add(size).ok_or_else(|| {
        CodecError::Message("NCSD partition layout overflows the image".to_owned())
      })?;
    }

    Ok((table, running))
  }

  fn header_bytes(
    metadata: &NcsdMetadata,
    table: &[(u32, u32); PARTITION_COUNT],
  ) -> Result<[u8; HEADER_SIZE], CodecError> {
    let signature = hex_field(&metadata.signature, "signature", 0x100)?;
    let exheader_hash = hex_field(&metadata.exheader_hash, "exheader hash", EXHEADER_HASH_SIZE)?;
    let header_reserved = hex_field(&metadata.header_reserved, "header reserved", RESERVED_SIZE)?;

    let mut header = [0u8; HEADER_SIZE];
    header[..0x100].copy_from_slice(&signature);
    header[MAGIC_OFFSET..MAGIC_OFFSET + NCSD_MAGIC.len()].copy_from_slice(NCSD_MAGIC);
    header[0x104..0x108].copy_from_slice(&metadata.image_size.to_le_bytes());
    header[0x108..0x110].copy_from_slice(&metadata.media_id.to_le_bytes());
    header[0x110..0x118].copy_from_slice(&metadata.partition_fs_types);
    header[0x118..0x120].copy_from_slice(&metadata.partition_crypto_types);

    for (index, (offset_units, size_units)) in table.iter().enumerate() {
      let base = PARTITION_TABLE_OFFSET + index * 8;
      header[base..base + 4].copy_from_slice(&offset_units.to_le_bytes());
      header[base + 4..base + 8].copy_from_slice(&size_units.to_le_bytes());
    }

    header[EXHEADER_HASH_OFFSET..EXHEADER_HASH_OFFSET + EXHEADER_HASH_SIZE]
      .copy_from_slice(&exheader_hash);
    header[0x180..0x184].copy_from_slice(&metadata.additional_header_size.to_le_bytes());
    header[0x184..0x188].copy_from_slice(&metadata.sector_zero_offset.to_le_bytes());
    header[0x188..0x190].copy_from_slice(&metadata.flags);

    for (index, id) in metadata.partition_ids.iter().enumerate() {
      let base = PARTITION_ID_TABLE_OFFSET + index * 8;
      header[base..base + 8].copy_from_slice(&id.to_le_bytes());
    }

    header[RESERVED_OFFSET..RESERVED_OFFSET + RESERVED_SIZE].copy_from_slice(&header_reserved);

    Ok(header)
  }
}

fn carries_ncch_magic(data: &[u8], partition: &PresentPartition) -> bool {
  partition
    .offset
    .checked_add(NCCH_MAGIC_OFFSET)
    .and_then(|offset| usize::try_from(offset).ok())
    .and_then(|offset| data.get(offset..))
    .is_some_and(|partition| partition.starts_with(NCCH_MAGIC))
}

fn partition_overflow(field: &str, index: usize) -> CodecError {
  CodecError::Message(format!(
    "NCSD partition {index} {field} overflows a 64-bit size"
  ))
}

fn hex_field(text: &str, name: &str, len: usize) -> Result<Vec<u8>, CodecError> {
  let bytes = hex::decode(text).map_err(|err| {
    CodecError::Message(format!(
      "NCSD metadata {name} is not valid hexadecimal: {err}"
    ))
  })?;

  if bytes.len() != len {
    return Err(CodecError::Message(format!(
      "NCSD metadata {name} is {} bytes, expected {len}",
      bytes.len()
    )));
  }

  Ok(bytes)
}
