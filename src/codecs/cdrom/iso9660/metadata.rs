//! What a rebuild needs to know about an ISO 9660 image beyond its files.

use crate::codecs::errors::CodecError;
use serde::{Deserialize, Serialize};

/// The name the metadata artifact is given.
pub(crate) const METADATA_NAME: &str = "ISO9660.json";

/// The metadata of an ISO 9660 image, as a rebuild needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IsoMetadata {
  /// The image's Volume Identifier, without the padding ISO 9660 mandates.
  pub volume_identifier: String,

  /// The image's files, in the order they sit on the disc.
  ///
  /// It must name every file artifact exactly once.
  pub file_order: Vec<String>,
}

impl IsoMetadata {
  pub(crate) fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the ISO metadata: {err}")))
  }

  pub(crate) fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the ISO metadata artifact: {err}"))
    })
  }
}
