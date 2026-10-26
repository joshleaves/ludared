use serde::Deserialize;
use thiserror::Error;

use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;

mod compress;
mod decompress;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrsError {
  #[error("Unexpected EOF")]
  UnexpectedEof,
  #[error("Invalid back reference")]
  InvalidBackReference,
}

const CODEC_ID: &str = "std/sega/compression/prs";
const CODEC_NAME: &str = "Sega PRS Decompression";
const CODEC_DESC: &str = r#"Codec description

Arguments (JSON):
  chain (default: 256)
    Maximum number of previous match candidates examined per input position
    during compression. Higher values may improve compression at the cost
    of speed.

Example:
  {"chain":256}
"#;

pub(crate) struct PrsCompression;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct PrsCodecArgs {
  pub(crate) chain: usize,
}

impl LudaredCodecArgs for PrsCodecArgs {}
impl Default for PrsCodecArgs {
  fn default() -> Self {
    Self { chain: 256 }
  }
}

impl Codec for PrsCompression {
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

  fn can_handle(&self, _: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    Ok(CodecHandlingConfidence::Possible)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("PRS".to_string())
  }

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], _: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    let decompressed = decompress::prs_decompress(data)
      .map_err(|e| CodecError::Message(format!("PRS Stream error: {e:?}")))?;

    Ok(vec![DecodedArtifact {
      name: "DECOMPRESSED".into(),
      data: decompressed,
    }])
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
    let args = PrsCodecArgs::from_args(args)?;
    if args.chain == 0 {
      return Err(CodecError::Message(
        "PRS Compression Error: Argument 'chain' must be greater than 0".into(),
      ));
    }
    if artifacts.len() != 1 {
      return Err(CodecError::Message(format!(
        "Invalid number of PRS streams: {} (expected 0)",
        artifacts.len()
      )));
    }

    let data = &artifacts[0].data;
    let compressed = compress::prs_compress(data, args.chain);

    Ok(compressed)
  }
}

#[cfg(test)]
mod tests {
  use super::compress::prs_compress;
  use super::decompress::prs_decompress;

  fn roundtrip(data: &[u8]) {
    let compressed = prs_compress(data, 256);
    let decompressed = prs_decompress(&compressed).expect("Valid PRS stream");

    assert_eq!(decompressed, data);
  }

  #[test]
  fn roundtrips_basic_inputs() {
    for data in [
      &b""[..],
      &b"A"[..],
      &b"AB"[..],
      &b"ABC"[..],
    ] {
      roundtrip(data);
    }

    roundtrip(&vec![0; 5000]);
    roundtrip(&b"AB".repeat(5000));
    roundtrip(&b"Sakura Taisen main menu 0123456789 ".repeat(300));
  }

  #[test]
  fn roundtrips_match_length_boundaries() {
    for len in [
      1,
      2,
      3,
      4,
      5,
      6,
      7,
      8,
      9,
      10,
      11,
      255,
      256,
      257,
      511,
      512,
      1023,
    ] {
      roundtrip(&vec![b'Z'; len]);
    }

    for len in [
      250_usize,
      255,
      256,
      257,
      260,
      300,
    ] {
      let data = b"AB".repeat(len.div_ceil(2));
      roundtrip(&data[..len]);
    }
  }

  #[test]
  fn roundtrips_distant_references() {
    let mut data = vec![b'Q'; 300];

    data.extend(b"\x01\x02".repeat(200));
    data.extend(vec![b'Q'; 300]);

    roundtrip(&data);
  }

  #[test]
  fn roundtrips_deterministic_noise() {
    let mut state: u32 = 12345;
    let mut data = Vec::with_capacity(4000);

    for _ in 0..4000 {
      state = state.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;

      data.push((state >> 16) as u8);
    }

    let compressed = prs_compress(&data, 256);

    assert!(
      compressed.len() >= data.len(),
      "Unexpected compression ratio: {} -> {}",
      data.len(),
      compressed.len()
    );

    assert_eq!(prs_decompress(&compressed).unwrap(), data);
  }

  #[test]
  fn empty_input_produces_a_valid_stream() {
    let compressed = prs_compress(b"", 256);

    assert!(!compressed.is_empty());
    assert_eq!(prs_decompress(&compressed).unwrap(), b"");
  }

  #[test]
  fn rejects_truncated_stream() {
    assert!(prs_decompress(&[0xff, 0xff, 0xff]).is_err());
  }
}
