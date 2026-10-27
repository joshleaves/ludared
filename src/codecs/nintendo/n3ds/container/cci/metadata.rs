//! NCSD metadata required to rebuild a CCI image.
use serde::Deserialize;
use serde::Serialize;

use crate::codecs::errors::CodecError;

pub(crate) const METADATA_NAME: &str = "NCSD.json";
pub(crate) const CARD_INFO_NAME: &str = "cardinfo.bin";
pub(crate) const TRAILER_NAME: &str = "trailer.bin";

pub(crate) fn partition_name(index: usize) -> String {
  format!("partition{index}.ncch")
}

pub(crate) fn partition_index(name: &str) -> Option<usize> {
  name
    .strip_prefix("partition")?
    .strip_suffix(".ncch")?
    .parse()
    .ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartitionMetadata {
  pub index: usize,
  pub offset: u32,
  pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NcsdMetadata {
  /// Preserved verbatim; cannot be recomputed without Nintendo's private key.
  pub signature: String,

  pub image_size: u32,
  pub media_id: u64,
  pub partition_fs_types: [u8; 8],
  pub partition_crypto_types: [u8; 8],
  pub flags: [u8; 8],
  pub exheader_hash: String,
  pub additional_header_size: u32,
  pub sector_zero_offset: u32,
  pub partition_ids: [u64; 8],
  pub header_reserved: String,

  /// Original layout; offsets and sizes are recalculated on rebuild.
  pub partitions: Vec<PartitionMetadata>,
}

impl NcsdMetadata {
  pub(crate) fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the NCSD metadata: {err}")))
  }

  pub(crate) fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the NCSD metadata artifact: {err}"))
    })
  }
}
