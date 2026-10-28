//! Nintendo 3DS NCCH (CTR Executable Image) codec.
//!
//! Splits an image into its header, its independently-placed regions and the
//! bytes no region claims, and rebuilds it from those artifacts alone. Regions
//! are carried whole and never decoded: ExeFS and RomFS stay opaque, encrypted
//! or not.
//!
//! # Hashes and signatures
//!
//! Every region hash in the header covers *cleartext*, so a hash is only
//! recomputed for a region stored in plaintext (NCCH flags byte 7, NoCrypto
//! bit set). An encrypted region keeps its recorded hash, and replacing one is
//! refused rather than written with a stale hash. The RSA signatures — the
//! header's, the exheader's and the access descriptor's — are carried verbatim
//! and never verified or regenerated, so a rebuilt image is structurally valid
//! but not guaranteed to boot on retail hardware.
//!
//! # Layout
//!
//! The header records where four regions sit, in media units, and fixes the
//! extended header at 0x200 with the access descriptor immediately after it.
//! Everything else — gaps between regions, trailing data — is unclaimed space.
//!
//! By default a decode keeps that space implicit: it emits no padding artifacts
//! and records the first unclaimed byte as `byte_padding`, defaulting to zero
//! when the image has no unclaimed interval. That keeps `byte_padding` a total
//! mode marker: every implicit decode records one, and an explicit decode always
//! records none. A rebuild keeps each section where it was, reserves its original
//! allocation, and fills any hole a shrink or an in-place resize opens with that
//! byte. A section that outgrows its allocation moves the sections after it.
//!
//! With `preserve_padding` a decode emits every unclaimed interval as a
//! `paddingN.bin` artifact instead. A rebuild then packs the sections
//! sequentially from their current artifact sizes, so resizing a section or a
//! padding interval shifts everything after it.

use crate::codecs::common::bytes::{
  read_field as field, read_u32_le as u32_le, read_u64_le as u64_le,
};
use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::n3ds::container::media_unit::media_unit_size;
use crate::codecs::{CodecHandlingConfidence, DecodedArtifact, LudaredCodecArgs};
use crate::hash::sha256_bytes as sha256_hex;
use log::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod metadata;

pub(crate) use metadata::{METADATA_NAME, NcchMetadata};
use metadata::{
  NcchFixedSection, NcchFixedSectionKind, NcchSection, NcchVariableSection,
  NcchVariableSectionKind, NcchVariant, section_name,
};

const HEADER_SIZE: usize = 0x200;
const MAGIC_OFFSET: usize = 0x100;
const NCCH_MAGIC: &[u8; 4] = b"NCCH";

/// The access descriptor's fixed size; it always follows the exheader.
const ACCESS_DESCRIPTOR_SIZE: u64 = 0x400;

const EXHEADER_SIZE_OFFSET: usize = 0x180;
const PLAIN_OFFSET: usize = 0x190;
const PLAIN_SIZE: usize = 0x194;
const LOGO_OFFSET: usize = 0x198;
const LOGO_SIZE: usize = 0x19C;
const EXEFS_OFFSET: usize = 0x1A0;
const EXEFS_SIZE: usize = 0x1A4;
const EXEFS_HASH_REGION_SIZE: usize = 0x1A8;
const ROMFS_OFFSET: usize = 0x1B0;
const ROMFS_SIZE: usize = 0x1B4;
const ROMFS_HASH_REGION_SIZE: usize = 0x1B8;

const FLAG_MEDIA_UNIT: usize = 6;
const FLAG_OTHER: usize = 7;
const OTHER_NO_CRYPTO: u8 = 0x4;

/// Parser/rebuilder shared by the public CXI and CFA variant codecs.
pub(crate) struct NcchEngine;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NcchCodecArgs {
  /// When true, extract unclaimed intervals as paddingN.bin artifacts.
  /// When false (default), use implicit padding with byte_padding.
  #[serde(default)]
  preserve_padding: bool,
}
impl LudaredCodecArgs for NcchCodecArgs {}

#[cfg(test)]
pub(crate) mod testkit {
  use super::*;

  pub(crate) const UNIT: usize = 0x200;

  /// Builds a deterministic plaintext CXI or CFA for adapter tests.
  pub(crate) fn build(exheader_size: usize, regions: &[(&str, u32, u32)]) -> Vec<u8> {
    let unit = UNIT;
    let access_size = if exheader_size == 0 { 0 } else { 0x400 };
    let end = regions
      .iter()
      .map(|(_, offset, size)| (*offset as usize + *size as usize) * unit)
      .max()
      .unwrap_or(HEADER_SIZE + exheader_size + access_size)
      .max(HEADER_SIZE + exheader_size + access_size);
    let mut image = vec![0u8; end];
    image[0x100..0x104].copy_from_slice(NCCH_MAGIC);
    image[0x104..0x108].copy_from_slice(&((end / unit) as u32).to_le_bytes());
    image[0x108..0x110].copy_from_slice(&0x0004_0000_1234_5678u64.to_le_bytes());
    image[0x118..0x120].copy_from_slice(&0x0004_0000_1234_5678u64.to_le_bytes());
    image[0x180..0x184].copy_from_slice(&(exheader_size as u32).to_le_bytes());
    image[0x188] = 0; // 0x200-byte media units
    image[0x18F] = OTHER_NO_CRYPTO;

    for (name, offset, size) in regions {
      let (field, hash_region_field) = match *name {
        "plain.bin" => (PLAIN_OFFSET, None),
        "logo.bin" => (LOGO_OFFSET, None),
        "exefs.bin" => (EXEFS_OFFSET, Some(EXEFS_HASH_REGION_SIZE)),
        "romfs.bin" => (ROMFS_OFFSET, Some(ROMFS_HASH_REGION_SIZE)),
        _ => panic!("unknown test region {name}"),
      };
      image[field..field + 4].copy_from_slice(&offset.to_le_bytes());
      image[field + 4..field + 8].copy_from_slice(&size.to_le_bytes());
      if let Some(hash_field) = hash_region_field {
        image[hash_field..hash_field + 4].copy_from_slice(&1u32.to_le_bytes());
      }
      let start = *offset as usize * unit;
      let finish = start + *size as usize * unit;
      for (index, byte) in image[start..finish].iter_mut().enumerate() {
        *byte = (index as u8).wrapping_add(name.as_bytes()[0]);
      }
      let hash = match *name {
        "logo.bin" => Some((0x130, super::digest(&image[start..finish]))),
        "exefs.bin" => Some((
          0x1C0,
          super::digest(&image[start..start + unit.min(finish - start)]),
        )),
        "romfs.bin" => Some((
          0x1E0,
          super::digest(&image[start..start + unit.min(finish - start)]),
        )),
        _ => None,
      };
      if let Some((hash_offset, hash_bytes)) = hash {
        image[hash_offset..hash_offset + 0x20].copy_from_slice(&hash_bytes);
      }
    }
    if exheader_size > 0 {
      let exheader_end = HEADER_SIZE + exheader_size;
      let exheader_hash = super::digest(&image[HEADER_SIZE..exheader_end]);
      image[0x160..0x180].copy_from_slice(&exheader_hash);
    }
    image
  }
}

impl NcchEngine {
  /// Detects an NCCH image by its header.
  ///
  /// The bytes decide, never a file name: the 'NCCH' magic has to open the
  /// header, and the regions it declares have to sit inside the declared
  /// content size without overlapping. Region hashes are deliberately not
  /// checked here — they cover cleartext and would fail on encrypted content.
  pub(crate) fn can_handle(
    data: &[u8],
    expected: Option<NcchVariant>,
  ) -> Result<CodecHandlingConfidence, CodecError> {
    let Ok(header) = NcchHeader::parse(data) else {
      return Ok(CodecHandlingConfidence::No);
    };
    if data.len() as u64 > header.capacity()? {
      return Ok(CodecHandlingConfidence::No);
    }
    let variant = header.variant()?;
    if expected.is_some_and(|wanted| wanted != variant) {
      return Ok(CodecHandlingConfidence::No);
    }
    match header.layout(data) {
      Ok(layout)
        if layout
          .iter()
          .any(|(section, _)| section.kind != NcchVariableSectionKind::Padding) =>
      {
        Ok(if expected.is_some() {
          CodecHandlingConfidence::Certain
        } else {
          CodecHandlingConfidence::Likely
        })
      }
      _ => Ok(CodecHandlingConfidence::No),
    }
  }

  /// Slices an image into its metadata and the sections the header describes.
  ///
  /// The metadata leads, so the manifest records it first. Sections are handed
  /// over whole in image order. With implicit padding the bytes no region claims
  /// stay in the image and only the first of them is recorded; with explicit
  /// padding every such interval becomes a `paddingN.bin` artifact.
  pub(crate) fn decode(
    data: &[u8],
    args: Option<&str>,
    expected: Option<NcchVariant>,
  ) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding Nintendo 3DS NCCH image");
    let args = NcchCodecArgs::from_args(args)?;
    let variant = NcchHeader::parse(data)?.variant()?;
    if expected.is_some_and(|wanted| wanted != variant) {
      return Err(CodecError::Message(format!(
        "NCCH variant is {variant:?}, not the requested {expected:?}"
      )));
    }

    let header = NcchHeader::parse(data)?;
    let capacity = header.capacity()?;
    if data.len() as u64 > capacity {
      return Err(CodecError::Message(format!(
        "NCCH image is {} bytes, past its declared content size of {capacity:#x}",
        data.len()
      )));
    }
    let layout = header.layout(data)?;

    let mut metadata = header.metadata(&layout);
    let mut artifacts = Vec::with_capacity(layout.len() + 1);
    // Filled in once the sections have been walked.
    artifacts.push(DecodedArtifact {
      name: METADATA_NAME.to_owned(),
      data: Vec::new(),
    });

    let mut padding_index = 0;
    let mut byte_padding = None;

    for (section, bytes) in &layout {
      match section.kind {
        NcchVariableSectionKind::Padding if !args.preserve_padding => {
          // Implicit padding: the interval stays part of the image and is not
          // extracted. Its first byte is what a rebuild uses for new gaps.
          if byte_padding.is_none() && !bytes.is_empty() {
            byte_padding = Some(bytes[0]);
          }
        }
        NcchVariableSectionKind::Padding => {
          artifacts.push(DecodedArtifact {
            name: section_name(section.kind, padding_index),
            data: bytes.to_vec(),
          });
          padding_index += 1;
        }
        kind => artifacts.push(DecodedArtifact {
          name: section_name(kind, 0),
          data: bytes.to_vec(),
        }),
      }
    }

    if !args.preserve_padding {
      // Implicit decoding always records a padding byte, even when the image
      // has no unclaimed interval: `byte_padding` is the mode marker, so a
      // gap-free image must not decode to the same metadata as an explicit one.
      metadata.byte_padding = Some(byte_padding.unwrap_or(DEFAULT_PADDING_BYTE));
    }

    artifacts[0].data = metadata.to_json()?;

    Ok(artifacts)
  }
  /// Rebuilds an image from the artifacts a decode produced.
  ///
  /// The strategy comes from the metadata: a recorded `byte_padding` keeps the
  /// implicit layout, with each section at its recorded position and allocation,
  /// while explicit padding packs the sections sequentially from their current
  /// artifact sizes. The decode arguments are deliberately not consulted.
  pub(crate) fn encode(
    artifacts: &[DecodedArtifact],
    expected: Option<NcchVariant>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding Nintendo 3DS NCCH image");
    let sorted = NcchEngine::sort_artifacts(artifacts)?;
    let metadata = NcchMetadata::from_json(sorted.metadata)?;
    let metadata_variant = if metadata
      .fixed_sections
      .iter()
      .any(|section| section.kind == NcchFixedSectionKind::Exheader)
    {
      NcchVariant::Cxi
    } else {
      NcchVariant::Cfa
    };
    if expected.is_some_and(|wanted| wanted != metadata_variant) {
      return Err(CodecError::Message(format!(
        "NCCH metadata describes {metadata_variant:?}, not the requested {expected:?}"
      )));
    }

    let unit = media_unit_size(metadata.flags[FLAG_MEDIA_UNIT]).ok_or_else(|| {
      CodecError::Message(format!(
        "NCCH media unit size flag {} overflows a 64-bit size",
        metadata.flags[FLAG_MEDIA_UNIT]
      ))
    })?;
    let no_crypto = metadata.flags[FLAG_OTHER] & OTHER_NO_CRYPTO != 0;

    // Sections in image order; the padding index is positional, so it is
    // assigned only once the order is known.
    let mut sections = metadata.variable_sections.clone();
    sections.sort_by_key(|section| section.position);
    let named = NcchEngine::name_sections(&sections)?;
    let explicit = NcchEngine::uses_explicit_padding(&metadata);
    NcchEngine::check_artifacts(&named, &sorted, explicit)?;

    let layout = if explicit {
      NcchEngine::layout_explicit(&named, &sorted)?
    } else {
      NcchEngine::layout_implicit(&named, &sorted)?
    };

    NcchEngine::validate_geometry(&layout, &metadata, unit)?;

    // Hashes over plaintext are recomputed; over ciphertext the recorded bytes
    // are kept and the region is required to be unchanged.
    let find = |kind| layout.sections.iter().find(|section| section.kind == kind);

    // The logo region is never encrypted, so its hash is always recomputable.
    let logo_hash = match find(NcchVariableSectionKind::LogoRegion) {
      Some(section) => digest(content_bytes(section)?),
      None => hex_field(&metadata.logo_hash, "logo hash")?,
    };

    let exheader_hash = match find(NcchVariableSectionKind::Exheader) {
      Some(section) if no_crypto => digest(content_bytes(section)?),
      Some(section) => {
        require_stored(
          &section.name,
          content_bytes(section)?,
          &metadata.exheader_stored_sha256,
        )?;
        hex_field(&metadata.exheader_hash, "exheader hash")?
      }
      None => hex_field(&metadata.exheader_hash, "exheader hash")?,
    };

    let exefs_hash = match find(NcchVariableSectionKind::Exefs) {
      // A zero hash-region size covers nothing, so there is nothing to recompute.
      Some(_) if metadata.exefs_hash_region_size == 0 => {
        hex_field(&metadata.exefs_hash, "ExeFS hash")?
      }
      Some(section) if no_crypto => superblock_hash(
        content_bytes(section)?,
        metadata.exefs_hash_region_size,
        unit,
      ),
      Some(section) => {
        require_stored(
          &section.name,
          content_bytes(section)?,
          &metadata.exefs_stored_sha256,
        )?;
        hex_field(&metadata.exefs_hash, "ExeFS hash")?
      }
      None => hex_field(&metadata.exefs_hash, "ExeFS hash")?,
    };

    let romfs_hash = match find(NcchVariableSectionKind::Romfs) {
      Some(_) if metadata.romfs_hash_region_size == 0 => {
        hex_field(&metadata.romfs_hash, "RomFS hash")?
      }
      Some(section) if no_crypto => superblock_hash(
        content_bytes(section)?,
        metadata.romfs_hash_region_size,
        unit,
      ),
      Some(section) => {
        require_stored(
          &section.name,
          content_bytes(section)?,
          &metadata.romfs_stored_sha256,
        )?;
        hex_field(&metadata.romfs_hash, "RomFS hash")?
      }
      None => hex_field(&metadata.romfs_hash, "RomFS hash")?,
    };

    let header = NcchEngine::header_bytes(
      &metadata,
      &layout.sections,
      unit,
      logo_hash,
      exheader_hash,
      exefs_hash,
      romfs_hash,
    )?;

    let image_len = usize::try_from(layout.image_len).map_err(|_| {
      CodecError::Message(format!(
        "NCCH image would be {} bytes, too large for this platform",
        layout.image_len
      ))
    })?;
    let fill = if explicit {
      0
    } else {
      metadata.byte_padding.unwrap_or(DEFAULT_PADDING_BYTE)
    };
    let mut image = vec![fill; image_len];
    image[..HEADER_SIZE].copy_from_slice(&header);
    for section in &layout.sections {
      let bytes = section.bytes;
      if let Some(bytes) = bytes {
        let start = usize::try_from(section.position).map_err(|_| {
          CodecError::Message(format!(
            "NCCH {} starts at {}, too large for this platform",
            section.name, section.position
          ))
        })?;
        let end = start.checked_add(bytes.len()).ok_or_else(|| {
          CodecError::Message(format!("NCCH {} overflows the image", section.name))
        })?;
        image[start..end].copy_from_slice(bytes);
      }
    }

    Ok(image)
  }
}

/// The NCCH header, field by field.
struct NcchHeader {
  signature: [u8; 0x100],
  content_size: u32,
  partition_id: u64,
  maker_code: u16,
  version: u16,
  seed: u32,
  program_id: u64,
  reserved_120: [u8; 0x10],
  product_code: [u8; 0x10],
  exheader_size: u32,
  exefs_hash_region_size: u32,
  romfs_hash_region_size: u32,
  reserved_184: u32,
  flags: [u8; 8],
  reserved_1ac: u32,
  reserved_1bc: u32,
  logo_hash: [u8; 0x20],
  exheader_hash: [u8; 0x20],
  exefs_hash: [u8; 0x20],
  romfs_hash: [u8; 0x20],
  plain_offset: u32,
  plain_size: u32,
  logo_offset: u32,
  logo_size: u32,
  exefs_offset: u32,
  exefs_size: u32,
  romfs_offset: u32,
  romfs_size: u32,
}

impl NcchHeader {
  fn parse(data: &[u8]) -> Result<Self, CodecError> {
    let header = data.get(..HEADER_SIZE).ok_or_else(|| {
      CodecError::Message(format!(
        "NCCH header is truncated: {} bytes (Minimum: {HEADER_SIZE})",
        data.len()
      ))
    })?;

    if header.get(MAGIC_OFFSET..MAGIC_OFFSET + NCCH_MAGIC.len()) != Some(NCCH_MAGIC.as_slice()) {
      return Err(CodecError::Message(
        "Not an NCCH image: the 'NCCH' magic is missing at 0x100".to_owned(),
      ));
    }

    Ok(Self {
      signature: field(header, 0)?,
      content_size: u32_le(header, 0x104)?,
      partition_id: u64_le(header, 0x108)?,
      maker_code: field::<2>(header, 0x110).map(u16::from_le_bytes)?,
      version: field::<2>(header, 0x112).map(u16::from_le_bytes)?,
      seed: u32_le(header, 0x114)?,
      program_id: u64_le(header, 0x118)?,
      reserved_120: field(header, 0x120)?,
      product_code: field(header, 0x150)?,
      exheader_size: u32_le(header, EXHEADER_SIZE_OFFSET)?,
      exefs_hash_region_size: u32_le(header, EXEFS_HASH_REGION_SIZE)?,
      romfs_hash_region_size: u32_le(header, ROMFS_HASH_REGION_SIZE)?,
      reserved_184: u32_le(header, 0x184)?,
      flags: field(header, 0x188)?,
      reserved_1ac: u32_le(header, 0x1AC)?,
      reserved_1bc: u32_le(header, 0x1BC)?,
      logo_hash: field(header, 0x130)?,
      exheader_hash: field(header, 0x160)?,
      exefs_hash: field(header, 0x1C0)?,
      romfs_hash: field(header, 0x1E0)?,
      plain_offset: u32_le(header, PLAIN_OFFSET)?,
      plain_size: u32_le(header, PLAIN_SIZE)?,
      logo_offset: u32_le(header, LOGO_OFFSET)?,
      logo_size: u32_le(header, LOGO_SIZE)?,
      exefs_offset: u32_le(header, EXEFS_OFFSET)?,
      exefs_size: u32_le(header, EXEFS_SIZE)?,
      romfs_offset: u32_le(header, ROMFS_OFFSET)?,
      romfs_size: u32_le(header, ROMFS_SIZE)?,
    })
  }

  fn variant(&self) -> Result<NcchVariant, CodecError> {
    if self.exheader_size > 0 {
      Ok(NcchVariant::Cxi)
    } else {
      Ok(NcchVariant::Cfa)
    }
  }

  fn media_unit(&self) -> Result<u64, CodecError> {
    media_unit_size(self.flags[FLAG_MEDIA_UNIT]).ok_or_else(|| {
      CodecError::Message(format!(
        "NCCH media unit size flag {} overflows a 64-bit size",
        self.flags[FLAG_MEDIA_UNIT]
      ))
    })
  }

  fn capacity(&self) -> Result<u64, CodecError> {
    u64::from(self.content_size)
      .checked_mul(self.media_unit()?)
      .ok_or_else(|| {
        CodecError::Message(format!(
          "NCCH content size {} media units overflows a 64-bit size",
          self.content_size
        ))
      })
  }

  /// The four regions the header places in media units, as (offset, size, name).
  fn declared_regions(&self) -> Vec<(u32, u32, &'static str)> {
    [
      (self.plain_offset, self.plain_size, "plain region"),
      (self.logo_offset, self.logo_size, "logo region"),
      (self.exefs_offset, self.exefs_size, "ExeFS"),
      (self.romfs_offset, self.romfs_size, "RomFS"),
    ]
    .into_iter()
    .filter(|(_, size, _)| *size > 0)
    .collect()
  }

  /// Works out every section of the image, padding included, in image order,
  /// each paired with the bytes it holds.
  ///
  /// The extended header sits at 0x200 with the access descriptor directly after
  /// it; the four unit-based regions sit where the header points. A region
  /// whose size is zero is absent, and the bytes no region claims become
  /// padding sections, so the sections cover the whole image.
  fn layout<'a>(&self, data: &'a [u8]) -> Result<Vec<(NcchSection, &'a [u8])>, CodecError> {
    let unit = self.media_unit()?;
    let mut regions: Vec<(u64, u64, &'static str)> = Vec::new();

    if self.exheader_size > 0 {
      regions.push((
        HEADER_SIZE as u64,
        u64::from(self.exheader_size),
        "exheader",
      ));
      regions.push((
        HEADER_SIZE as u64 + u64::from(self.exheader_size),
        ACCESS_DESCRIPTOR_SIZE,
        "access descriptor",
      ));
    }
    for (offset, size, name) in self.declared_regions() {
      regions.push((
        u64::from(offset)
          .checked_mul(unit)
          .ok_or_else(|| overflow("offset", name))?,
        u64::from(size)
          .checked_mul(unit)
          .ok_or_else(|| overflow("size", name))?,
        name,
      ));
    }
    regions.sort_by_key(|(offset, _, _)| *offset);

    let mut sections = Vec::new();
    let mut cursor = HEADER_SIZE as u64;

    for (offset, size, name) in regions {
      if offset < cursor {
        return Err(CodecError::Message(format!(
          "NCCH section starts at {offset:#x}, overlapping the section before it"
        )));
      }
      if offset > data.len() as u64 {
        return Err(CodecError::Message(format!(
          "NCCH {name} runs past the end of the {}-byte image",
          data.len()
        )));
      }
      if offset > cursor {
        sections.push((
          NcchSection {
            kind: NcchVariableSectionKind::Padding,
            offset: cursor,
            size: offset - cursor,
          },
          &data[cursor as usize..offset as usize],
        ));
      }
      let end = offset
        .checked_add(size)
        .ok_or_else(|| overflow("end", name))?;
      if end > data.len() as u64 {
        return Err(CodecError::Message(format!(
          "NCCH {name} runs past the end of the {}-byte image",
          data.len()
        )));
      }

      let kind = match name {
        "exheader" => NcchVariableSectionKind::Exheader,
        "access descriptor" => NcchVariableSectionKind::AccessDescriptor,
        "logo region" => NcchVariableSectionKind::LogoRegion,
        "plain region" => NcchVariableSectionKind::PlainRegion,
        "ExeFS" => NcchVariableSectionKind::Exefs,
        _ => NcchVariableSectionKind::Romfs,
      };
      sections.push((
        NcchSection { kind, offset, size },
        &data[offset as usize..end as usize],
      ));
      cursor = end;
    }

    if cursor < data.len() as u64 {
      sections.push((
        NcchSection {
          kind: NcchVariableSectionKind::Padding,
          offset: cursor,
          size: data.len() as u64 - cursor,
        },
        &data[cursor as usize..],
      ));
    }

    if sections
      .iter()
      .all(|(section, _)| section.kind == NcchVariableSectionKind::Padding)
    {
      return Err(CodecError::Message(
        "NCCH image declares no regions".to_owned(),
      ));
    }

    Ok(sections)
  }

  /// The metadata a rebuild needs, with each encryption-capable region's stored
  /// bytes fingerprinted so a replacement can be told from the original.
  fn metadata(&self, layout: &[(NcchSection, &[u8])]) -> NcchMetadata {
    let mut stored = (
      sha256_hex(&[]),
      sha256_hex(&[]),
      sha256_hex(&[]),
      sha256_hex(&[]),
      sha256_hex(&[]),
    );
    for (section, bytes) in layout {
      match section.kind {
        NcchVariableSectionKind::Exheader => stored.0 = sha256_hex(bytes),
        NcchVariableSectionKind::AccessDescriptor => stored.1 = sha256_hex(bytes),
        NcchVariableSectionKind::LogoRegion => stored.2 = sha256_hex(bytes),
        NcchVariableSectionKind::Exefs => stored.3 = sha256_hex(bytes),
        NcchVariableSectionKind::Romfs => stored.4 = sha256_hex(bytes),
        _ => {}
      }
    }
    NcchMetadata {
      signature: hex::encode(self.signature),
      content_size: self.content_size,
      partition_id: self.partition_id,
      maker_code: self.maker_code,
      version: self.version,
      seed: self.seed,
      program_id: self.program_id,
      reserved_120: self.reserved_120,
      product_code: self.product_code,
      exefs_hash_region_size: self.exefs_hash_region_size,
      romfs_hash_region_size: self.romfs_hash_region_size,
      reserved_184: self.reserved_184,
      flags: self.flags,
      reserved_1ac: self.reserved_1ac,
      reserved_1bc: self.reserved_1bc,
      byte_padding: None,
      fixed_sections: if self.exheader_size > 0 {
        vec![
          NcchFixedSection {
            kind: NcchFixedSectionKind::Exheader,
            size: u64::from(self.exheader_size),
          },
          NcchFixedSection {
            kind: NcchFixedSectionKind::AccessDescriptor,
            size: ACCESS_DESCRIPTOR_SIZE,
          },
        ]
      } else {
        Vec::new()
      },
      logo_hash: hex::encode(self.logo_hash),
      exheader_hash: hex::encode(self.exheader_hash),
      exefs_hash: hex::encode(self.exefs_hash),
      romfs_hash: hex::encode(self.romfs_hash),
      exheader_stored_sha256: stored.0,
      access_descriptor_stored_sha256: stored.1,
      exefs_stored_sha256: stored.3,
      romfs_stored_sha256: stored.4,
      variable_sections: layout
        .iter()
        .map(|(section, _bytes)| NcchVariableSection {
          kind: section.kind,
          position: section.offset,
          size_available: section.size,
        })
        .collect(),
    }
  }
}

/// The byte implicit padding falls back to when the source had no unclaimed
/// interval, so shrinking a section still has a defined fill.
const DEFAULT_PADDING_BYTE: u8 = 0;

struct SortedArtifacts<'a> {
  metadata: &'a [u8],
  sections: BTreeMap<String, &'a [u8]>,
}

/// A section placed at its final offset, with the bytes it contributes.
struct PlacedSection<'a> {
  kind: NcchVariableSectionKind,
  name: String,
  position: u64,
  size: u64,
  /// `None` for an implicit padding interval, which contributes only fill.
  bytes: Option<&'a [u8]>,
}

/// The final layout of a rebuild and the image length it covers.
struct NcchLayout<'a> {
  sections: Vec<PlacedSection<'a>>,
  image_len: u64,
}

impl NcchEngine {
  /// Sorts the artifacts into the metadata and the sections they name.
  ///
  /// Every name has to be one a decode produced, and no two may share it: an
  /// unknown artifact and a second copy of a known one are both holes or
  /// shadows in the image being rebuilt.
  fn sort_artifacts(artifacts: &[DecodedArtifact]) -> Result<SortedArtifacts<'_>, CodecError> {
    let mut metadata = None;
    let mut sections: BTreeMap<String, &[u8]> = BTreeMap::new();
    let mut seen = BTreeSet::new();

    for artifact in artifacts {
      if !seen.insert(artifact.name.as_str()) {
        return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
      }

      if artifact.name == METADATA_NAME {
        metadata = Some(artifact.data.as_slice());
      } else if section_kind(&artifact.name).is_some() {
        sections.insert(artifact.name.clone(), artifact.data.as_slice());
      } else {
        return Err(CodecError::Message(format!(
          "Unexpected artifact: {}",
          artifact.name
        )));
      }
    }

    let metadata = metadata.ok_or_else(|| {
      CodecError::Message(format!("Expected a {METADATA_NAME} artifact, found none"))
    })?;

    Ok(SortedArtifacts { metadata, sections })
  }

  /// Whether the metadata describes explicit padding.
  ///
  /// `byte_padding` is the only marker: an implicit decode always records one,
  /// including for a gap-free image, while an explicit decode records none.
  fn uses_explicit_padding(metadata: &NcchMetadata) -> bool {
    metadata.byte_padding.is_none()
  }

  /// Gives every section the artifact name a decode would have used.
  fn name_sections(
    sections: &[NcchVariableSection],
  ) -> Result<Vec<(NcchVariableSection, String)>, CodecError> {
    let mut named = Vec::with_capacity(sections.len());
    let mut seen = BTreeSet::new();
    let mut padding_index = 0;

    for section in sections {
      let name = if section.kind == NcchVariableSectionKind::Padding {
        let name = section_name(section.kind, padding_index);
        padding_index += 1;
        name
      } else {
        section_name(section.kind, 0)
      };

      if !seen.insert(name.clone()) {
        return Err(CodecError::DuplicateArtifact(name));
      }
      named.push((section.clone(), name));
    }

    Ok(named)
  }

  /// Checks that the artifacts and the metadata sections match one another.
  fn check_artifacts(
    named: &[(NcchVariableSection, String)],
    sorted: &SortedArtifacts<'_>,
    explicit: bool,
  ) -> Result<(), CodecError> {
    for (section, name) in named {
      if section.kind == NcchVariableSectionKind::Padding && !explicit {
        continue;
      }
      if !sorted.sections.contains_key(name) {
        return Err(CodecError::Message(format!(
          "Expected a {name} artifact, found none"
        )));
      }
    }

    for name in sorted.sections.keys() {
      let expected = named.iter().any(|(section, section_name)| {
        section_name == name && (explicit || section.kind != NcchVariableSectionKind::Padding)
      });
      if !expected {
        return Err(CodecError::Message(format!("Unexpected artifact: {name}")));
      }
    }

    Ok(())
  }

  /// Places the sections where the metadata recorded them, keeping each
  /// allocation and moving later sections only when one outgrows it.
  fn layout_implicit<'a>(
    named: &[(NcchVariableSection, String)],
    sorted: &'a SortedArtifacts<'_>,
  ) -> Result<NcchLayout<'a>, CodecError> {
    let mut placed = Vec::with_capacity(named.len());
    let mut expected = HEADER_SIZE as u64;
    let mut drift = 0u64;

    for (section, name) in named {
      if section.position != expected {
        return Err(CodecError::Message(format!(
          "NCCH section starts at {:#x}, but the previous section ends at {expected:#x}: \
           the recorded layout must cover the image without gaps",
          section.position
        )));
      }

      let position = section
        .position
        .checked_add(drift)
        .ok_or_else(|| overflow("offset", name))?;

      if section.kind == NcchVariableSectionKind::Padding {
        if section.size_available == 0 {
          return Err(CodecError::Message(format!(
            "NCCH {name} is empty, which no gap can be"
          )));
        }
        placed.push(PlacedSection {
          kind: section.kind,
          name: name.clone(),
          position,
          size: section.size_available,
          bytes: None,
        });
      } else {
        let bytes =
          sorted.sections.get(name).copied().ok_or_else(|| {
            CodecError::Message(format!("Expected a {name} artifact, found none"))
          })?;
        let size = bytes.len() as u64;
        if size == 0 {
          return Err(CodecError::Message(format!("NCCH {name} is empty")));
        }

        if size > section.size_available {
          drift = drift
            .checked_add(size - section.size_available)
            .ok_or_else(|| overflow("size", name))?;
        }

        placed.push(PlacedSection {
          kind: section.kind,
          name: name.clone(),
          position,
          size,
          bytes: Some(bytes),
        });
      }

      expected = section
        .position
        .checked_add(section.size_available)
        .ok_or_else(|| overflow("size", name))?;
    }

    Ok(NcchLayout {
      sections: placed,
      image_len: expected
        .checked_add(drift)
        .ok_or_else(|| CodecError::Message("NCCH image size overflows".to_owned()))?,
    })
  }

  /// Packs every section, padding included, from the end of the header using
  /// its current artifact size.
  fn layout_explicit<'a>(
    named: &[(NcchVariableSection, String)],
    sorted: &'a SortedArtifacts<'_>,
  ) -> Result<NcchLayout<'a>, CodecError> {
    let mut placed = Vec::with_capacity(named.len());
    let mut cursor = HEADER_SIZE as u64;

    for (section, name) in named {
      let bytes = sorted
        .sections
        .get(name)
        .copied()
        .ok_or_else(|| CodecError::Message(format!("Expected a {name} artifact, found none")))?;
      let size = bytes.len() as u64;
      if size == 0 && section.kind != NcchVariableSectionKind::Padding {
        return Err(CodecError::Message(format!("NCCH {name} is empty")));
      }

      placed.push(PlacedSection {
        kind: section.kind,
        name: name.clone(),
        position: cursor,
        size,
        bytes: Some(bytes),
      });
      cursor = cursor
        .checked_add(size)
        .ok_or_else(|| overflow("size", name))?;
    }

    Ok(NcchLayout {
      sections: placed,
      image_len: cursor,
    })
  }

  /// Checks the fixed regions, the media-unit alignment and the declared size.
  fn validate_geometry(
    layout: &NcchLayout<'_>,
    metadata: &NcchMetadata,
    unit: u64,
  ) -> Result<(), CodecError> {
    let exheader = layout
      .sections
      .iter()
      .find(|section| section.kind == NcchVariableSectionKind::Exheader);
    let access = layout
      .sections
      .iter()
      .find(|section| section.kind == NcchVariableSectionKind::AccessDescriptor);
    let fixed_exheader = metadata
      .fixed_sections
      .iter()
      .find(|section| section.kind == NcchFixedSectionKind::Exheader);
    let fixed_access = metadata
      .fixed_sections
      .iter()
      .find(|section| section.kind == NcchFixedSectionKind::AccessDescriptor);

    match (fixed_exheader, exheader) {
      (Some(fixed), Some(section)) => {
        if section.position != HEADER_SIZE as u64 {
          return Err(CodecError::Message(format!(
            "NCCH exheader starts at {:#x}, not directly after the header",
            section.position
          )));
        }
        if section.size != fixed.size {
          return Err(CodecError::Message(format!(
            "NCCH exheader is {:#x} bytes, but its fixed geometry is {:#x}",
            section.size, fixed.size
          )));
        }
      }
      (Some(_), None) => {
        return Err(CodecError::Message(
          "NCCH metadata records an exheader, but no exheader artifact was supplied".to_owned(),
        ));
      }
      (None, Some(_)) => {
        return Err(CodecError::Message(
          "NCCH metadata has an exheader artifact but no fixed exheader geometry".to_owned(),
        ));
      }
      (None, None) => {}
    }

    match (fixed_access, access) {
      (Some(fixed), Some(section)) => {
        let exheader_size = exheader.map_or(0, |section| section.size);
        if fixed.size != ACCESS_DESCRIPTOR_SIZE || section.size != ACCESS_DESCRIPTOR_SIZE {
          return Err(CodecError::Message(format!(
            "NCCH access descriptor must be {ACCESS_DESCRIPTOR_SIZE:#x} bytes"
          )));
        }
        if section.position != HEADER_SIZE as u64 + exheader_size {
          return Err(CodecError::Message(format!(
            "NCCH access descriptor starts at {:#x}, not directly after the exheader",
            section.position
          )));
        }
      }
      (Some(_), None) => {
        return Err(CodecError::Message(
          "NCCH metadata records an access descriptor, but no artifact was supplied".to_owned(),
        ));
      }
      (None, Some(_)) => {
        return Err(CodecError::Message(
          "NCCH metadata has an access descriptor artifact but no fixed geometry".to_owned(),
        ));
      }
      (None, None) => {}
    }

    if fixed_access.is_some() && fixed_exheader.is_none() {
      return Err(CodecError::Message(
        "NCCH access descriptor has no exheader to follow".to_owned(),
      ));
    }

    let mut regions = 0usize;
    for section in &layout.sections {
      if !matches!(
        section.kind,
        NcchVariableSectionKind::LogoRegion
          | NcchVariableSectionKind::PlainRegion
          | NcchVariableSectionKind::Exefs
          | NcchVariableSectionKind::Romfs
      ) {
        continue;
      }
      if section.size == 0 {
        return Err(CodecError::Message(format!(
          "NCCH {} is empty, which no region can be",
          section.name
        )));
      }
      for (value, what) in [
        (section.position, "offset"),
        (section.size, "size"),
      ] {
        if value % unit != 0 {
          return Err(CodecError::Message(format!(
            "NCCH section {what} is {value:#x}, not a whole number of {unit:#x}-byte media units"
          )));
        }
        if value / unit > u64::from(u32::MAX) {
          return Err(CodecError::Message(format!(
            "NCCH section {what} is {value:#x}, too large for the header's media-unit field"
          )));
        }
      }
      regions += 1;
    }

    if regions == 0 {
      return Err(CodecError::Message(
        "NCCH metadata describes no regions".to_owned(),
      ));
    }

    let capacity = u64::from(metadata.content_size)
      .checked_mul(unit)
      .ok_or_else(|| {
        CodecError::Message(format!(
          "NCCH content size {} media units overflows a 64-bit size",
          metadata.content_size
        ))
      })?;
    if layout.image_len > capacity {
      return Err(CodecError::Message(format!(
        "NCCH image would be {:#x} bytes, past the declared content size of {capacity:#x}",
        layout.image_len
      )));
    }

    Ok(())
  }

  /// The 0x200 bytes of the header a metadata and its layout describe.
  fn header_bytes(
    metadata: &NcchMetadata,
    placed: &[PlacedSection<'_>],
    unit: u64,
    logo_hash: [u8; 0x20],
    exheader_hash: [u8; 0x20],
    exefs_hash: [u8; 0x20],
    romfs_hash: [u8; 0x20],
  ) -> Result<[u8; HEADER_SIZE], CodecError> {
    let signature: [u8; 0x100] = hex_field(&metadata.signature, "signature")?;

    let mut header = [0u8; HEADER_SIZE];
    header[..0x100].copy_from_slice(&signature);
    header[MAGIC_OFFSET..MAGIC_OFFSET + NCCH_MAGIC.len()].copy_from_slice(NCCH_MAGIC);
    header[0x104..0x108].copy_from_slice(&metadata.content_size.to_le_bytes());
    header[0x108..0x110].copy_from_slice(&metadata.partition_id.to_le_bytes());
    header[0x110..0x112].copy_from_slice(&metadata.maker_code.to_le_bytes());
    header[0x112..0x114].copy_from_slice(&metadata.version.to_le_bytes());
    header[0x114..0x118].copy_from_slice(&metadata.seed.to_le_bytes());
    header[0x118..0x120].copy_from_slice(&metadata.program_id.to_le_bytes());
    header[0x120..0x130].copy_from_slice(&metadata.reserved_120);
    header[0x130..0x150].copy_from_slice(&logo_hash);
    header[0x150..0x160].copy_from_slice(&metadata.product_code);
    header[0x160..0x180].copy_from_slice(&exheader_hash);
    header[EXHEADER_SIZE_OFFSET..EXHEADER_SIZE_OFFSET + 4]
      .copy_from_slice(&exheader_section(placed)?.to_le_bytes());
    header[0x184..0x188].copy_from_slice(&metadata.reserved_184.to_le_bytes());
    header[0x188..0x190].copy_from_slice(&metadata.flags);

    for (offset, size, kind) in [
      (
        PLAIN_OFFSET,
        PLAIN_SIZE,
        NcchVariableSectionKind::PlainRegion,
      ),
      (LOGO_OFFSET, LOGO_SIZE, NcchVariableSectionKind::LogoRegion),
      (EXEFS_OFFSET, EXEFS_SIZE, NcchVariableSectionKind::Exefs),
      (ROMFS_OFFSET, ROMFS_SIZE, NcchVariableSectionKind::Romfs),
    ] {
      let section = placed.iter().find(|section| section.kind == kind);
      let offset_units = match section {
        Some(section) => units(section.position, unit, "offset")?,
        None => 0,
      };
      let size_units = match section {
        Some(section) => units(section.size, unit, "size")?,
        None => 0,
      };
      header[offset..offset + 4].copy_from_slice(&offset_units.to_le_bytes());
      header[size..size + 4].copy_from_slice(&size_units.to_le_bytes());
    }

    header[EXEFS_HASH_REGION_SIZE..EXEFS_HASH_REGION_SIZE + 4]
      .copy_from_slice(&metadata.exefs_hash_region_size.to_le_bytes());
    header[0x1AC..0x1B0].copy_from_slice(&metadata.reserved_1ac.to_le_bytes());
    header[ROMFS_HASH_REGION_SIZE..ROMFS_HASH_REGION_SIZE + 4]
      .copy_from_slice(&metadata.romfs_hash_region_size.to_le_bytes());
    header[0x1BC..0x1C0].copy_from_slice(&metadata.reserved_1bc.to_le_bytes());
    header[0x1C0..0x1E0].copy_from_slice(&exefs_hash);
    header[0x1E0..0x200].copy_from_slice(&romfs_hash);

    Ok(header)
  }
}

/// The bytes a placed content section contributes.
fn content_bytes<'a>(section: &PlacedSection<'a>) -> Result<&'a [u8], CodecError> {
  section
    .bytes
    .ok_or_else(|| CodecError::Message(format!("NCCH {} has no artifact", section.name)))
}

/// The extended header's recorded size.
fn exheader_section(placed: &[PlacedSection<'_>]) -> Result<u32, CodecError> {
  match placed
    .iter()
    .find(|section| section.kind == NcchVariableSectionKind::Exheader)
  {
    Some(section) => u32::try_from(section.size).map_err(|_| {
      CodecError::Message(format!(
        "NCCH exheader is {:#x} bytes, too large for the header field",
        section.size
      ))
    }),
    None => Ok(0),
  }
}

/// A byte count as a count of whole media units that fit the header field.
fn units(value: u64, unit: u64, what: &str) -> Result<u32, CodecError> {
  u32::try_from(value / unit).map_err(|_| {
    CodecError::Message(format!(
      "NCCH section {what} is {value:#x}, too large for the header's media-unit field"
    ))
  })
}

fn section_kind(name: &str) -> Option<NcchVariableSectionKind> {
  match name {
    "exheader.bin" => Some(NcchVariableSectionKind::Exheader),
    "access_descriptor.bin" => Some(NcchVariableSectionKind::AccessDescriptor),
    "logo_region.bin" => Some(NcchVariableSectionKind::LogoRegion),
    "plain_region.bin" => Some(NcchVariableSectionKind::PlainRegion),
    "exefs.bin" => Some(NcchVariableSectionKind::Exefs),
    "romfs.bin" => Some(NcchVariableSectionKind::Romfs),
    name => {
      let index = name.strip_prefix("padding")?.strip_suffix(".bin")?;
      index.parse::<usize>().ok()?;
      Some(NcchVariableSectionKind::Padding)
    }
  }
}

/// The bytes a hexadecimal metadata field stands for, of the expected length.
fn hex_field<const N: usize>(text: &str, name: &str) -> Result<[u8; N], CodecError> {
  let bytes = hex::decode(text).map_err(|err| {
    CodecError::Message(format!(
      "NCCH metadata {name} is not valid hexadecimal: {err}"
    ))
  })?;

  bytes.try_into().map_err(|bytes: Vec<u8>| {
    CodecError::Message(format!(
      "NCCH metadata {name} is {} bytes, expected {N}",
      bytes.len()
    ))
  })
}

/// The SHA-256 of a region's bytes.
fn digest(bytes: &[u8]) -> [u8; 0x20] {
  let mut hasher = Sha256::new();
  hasher.update(bytes);
  hasher.finalize().into()
}

/// The hash of the first `hash_region_size` media units of a region, clamped
/// to what the region actually holds.
fn superblock_hash(bytes: &[u8], hash_region_size: u32, unit: u64) -> [u8; 0x20] {
  let covered = u64::from(hash_region_size).min(bytes.len() as u64 / unit) * unit;
  digest(&bytes[..covered as usize])
}

/// Refuses a rebuild that changes a region stored as ciphertext: its recorded
/// hash covers bytes this codec cannot see, so a change would leave it stale.
fn require_stored(name: &str, bytes: &[u8], recorded: &str) -> Result<(), CodecError> {
  if sha256_hex(bytes) != recorded {
    return Err(CodecError::Message(format!(
      "NCCH {name} is encrypted: its hash covers the decrypted bytes, so it cannot be \
       replaced or resized without decryption"
    )));
  }

  Ok(())
}

fn overflow(field: &str, name: &str) -> CodecError {
  CodecError::Message(format!("NCCH {name} {field} overflows a 64-bit size"))
}

#[cfg(test)]
mod tests;
