//! NCCH metadata required to rebuild an image: every header field, the region
//! hashes, and the layout the regions sit in.

use crate::codecs::errors::CodecError;
use serde::{Deserialize, Serialize};

pub(crate) const METADATA_NAME: &str = "NCCH.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NcchVariant {
  Cxi,
  Cfa,
}

/// The kind of a variable section in the NCCH image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NcchVariableSectionKind {
  Exheader,
  AccessDescriptor,
  LogoRegion,
  PlainRegion,
  Exefs,
  Romfs,
  Padding,
}

/// One variable section, placed where the header or the gaps put it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NcchVariableSection {
  /// The kind of this section.
  pub kind: NcchVariableSectionKind,
  /// Where the section originally sat, in bytes from the start of the image.
  pub position: u64,
  /// How many bytes were originally allocated for this section.
  pub size_available: u64,
}

/// The fixed sections that have structural placement constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NcchFixedSectionKind {
  Exheader,
  AccessDescriptor,
}

/// Create a section artifact name from a kind and index.
///
/// For padding kinds, produces `paddingN.bin`. For other kinds, produces
/// the conventional `.bin` suffix names.
pub(crate) fn section_name(kind: NcchVariableSectionKind, index: usize) -> String {
  match kind {
    NcchVariableSectionKind::Exheader => "exheader.bin".to_owned(),
    NcchVariableSectionKind::AccessDescriptor => "access_descriptor.bin".to_owned(),
    NcchVariableSectionKind::LogoRegion => "logo_region.bin".to_owned(),
    NcchVariableSectionKind::PlainRegion => "plain_region.bin".to_owned(),
    NcchVariableSectionKind::Exefs => "exefs.bin".to_owned(),
    NcchVariableSectionKind::Romfs => "romfs.bin".to_owned(),
    NcchVariableSectionKind::Padding => format!("padding{}.bin", index),
  }
}

/// The metadata required to rebuild an image: every header field, the region
/// hashes, and the layout the regions sit in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NcchMetadata {
  /// Preserved verbatim; cannot be recomputed without Nintendo's private key.
  pub signature: String,

  /// Declared size of the whole image, in media units.
  pub content_size: u32,
  pub partition_id: u64,
  pub maker_code: u16,
  pub version: u16,
  pub seed: u32,
  pub program_id: u64,
  pub reserved_120: [u8; 0x10],
  pub product_code: [u8; 0x10],
  pub exefs_hash_region_size: u32,
  pub romfs_hash_region_size: u32,
  pub reserved_184: u32,
  pub flags: [u8; 8],
  pub reserved_1ac: u32,
  pub reserved_1bc: u32,

  /// Recorded region hashes, kept verbatim unless the region is re-hashed.
  pub logo_hash: String,
  pub exheader_hash: String,
  pub exefs_hash: String,
  pub romfs_hash: String,

  /// SHA-256 of each encryption-capable region as stored, ciphertext included:
  /// the only way to tell such a region from its replacement without decrypting.
  pub exheader_stored_sha256: String,
  pub access_descriptor_stored_sha256: String,
  pub exefs_stored_sha256: String,
  pub romfs_stored_sha256: String,

  /// The byte value used for implicit padding; always recorded by an implicit
  /// decode, including when the image has no unclaimed interval. If null, the
  /// metadata describes explicit padding instead.
  pub byte_padding: Option<u8>,

  /// The fixed sections with structural placement constraints.
  pub fixed_sections: Vec<NcchFixedSection>,

  /// The variable sections in image order.
  pub variable_sections: Vec<NcchVariableSection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NcchFixedSection {
  pub kind: NcchFixedSectionKind,
  pub size: u64,
}

/// One section of an NCCH image, with its kind, offset, and size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct NcchSection {
  /// The kind of this section.
  pub kind: NcchVariableSectionKind,
  /// Where the section sits, in bytes from the start of the image.
  pub offset: u64,
  /// The size of the section in bytes.
  pub size: u64,
}

impl NcchMetadata {
  pub(crate) fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the NCCH metadata: {err}")))
  }

  pub(crate) fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the NCCH metadata artifact: {err}"))
    })
  }
}
