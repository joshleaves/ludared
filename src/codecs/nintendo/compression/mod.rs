//! Nintendo compression formats.
//!
//! LZ10 and LZ11 share the same LZSS engine and differ only in their reference
//! encoding; their codecs also share the optional archive container argument.

pub(crate) mod lz10;
pub(crate) mod lz11;
mod lzss;

use serde::Deserialize;

use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::container::darc::Darc;
use crate::codecs::nintendo::container::sarc::Sarc;
use crate::codecs::{Codec, CodecHandlingConfidence, DecodedArtifact, LudaredCodecArgs};
use lzss::Format;

/// What the decompressed stream should be interpreted as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Container {
  /// Leave the decompressed bytes as a single artifact.
  #[default]
  Raw,
  /// Decode the decompressed bytes as a DARC archive.
  Darc,
  /// Decode the decompressed bytes as a SARC archive.
  Sarc,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompressionArgs {
  #[serde(default)]
  pub(crate) container: Container,
}

impl LudaredCodecArgs for CompressionArgs {}

pub(crate) fn can_handle(format: Format, data: &[u8]) -> CodecHandlingConfidence {
  if data.len() >= 4 && data[0] == format.magic() {
    CodecHandlingConfidence::Likely
  } else {
    CodecHandlingConfidence::No
  }
}

/// Resolves a preset into arguments for this codec family.
pub(crate) fn preset_args(
  preset: &str,
  args: Option<&str>,
  codec_id: &str,
) -> Result<String, CodecError> {
  let preset = match preset {
    "darc" => r#"{"container":"darc"}"#,
    "sarc" => r#"{"container":"sarc"}"#,
    _ => {
      return Err(CodecError::UndefinedPreset(
        preset.to_owned(),
        codec_id.to_owned(),
      ));
    }
  };
  Ok(CompressionArgs::merge(preset, args)?)
}

pub(crate) fn decode(
  format: Format,
  data: &[u8],
  args: Option<&str>,
) -> Result<Vec<DecodedArtifact>, CodecError> {
  let args = CompressionArgs::from_args(args)?;
  let decompressed = lzss::decompress(data, format)?;

  match args.container {
    Container::Raw => Ok(vec![DecodedArtifact {
      name: "DECOMPRESSED".to_owned(),
      data: decompressed,
    }]),
    Container::Darc => Darc.decode(&decompressed, None),
    Container::Sarc => Sarc.decode(&decompressed, None),
  }
}

pub(crate) fn encode(
  format: Format,
  artifacts: &[DecodedArtifact],
  args: Option<&str>,
) -> Result<Vec<u8>, CodecError> {
  let args = CompressionArgs::from_args(args)?;

  let wrapped;
  let artifacts = match args.container {
    Container::Raw => artifacts,
    Container::Darc => {
      wrapped = [DecodedArtifact {
        name: "DECOMPRESSED".to_owned(),
        data: Darc.encode(Vec::new(), artifacts, None)?,
      }];
      &wrapped
    }
    Container::Sarc => {
      wrapped = [DecodedArtifact {
        name: "DECOMPRESSED".to_owned(),
        data: Sarc.encode(Vec::new(), artifacts, None)?,
      }];
      &wrapped
    }
  };

  if artifacts.len() != 1 {
    return Err(CodecError::Message(format!(
      "{} codec expects exactly one artifact, found {}",
      format.label(),
      artifacts.len()
    )));
  }

  lzss::compress(&artifacts[0].data, format)
}
