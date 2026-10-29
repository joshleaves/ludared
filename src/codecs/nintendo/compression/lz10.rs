//! Nintendo LZ10 (LZSS) compression codec.

use crate::codecs::nintendo::compression::{self, lzss::Format};
use crate::codecs::{
  Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact, errors::CodecError,
};

const CODEC_ID: &str = "std/nintendo/compression/lz10";
const CODEC_NAME: &str = "Nintendo LZ10 compression";
const CODEC_DESC: &str = r#"Nintendo LZ10 (LZSS) compressed stream

Decompresses an LZ10 stream into a single DECOMPRESSED artifact, or passes the
decompressed bytes to an archive codec when the 'container' argument names one.

The stream is a four-byte header (0x10 and the 24-bit little-endian
decompressed size) followed by flag bytes and literal or back-reference
tokens. References are 3 to 18 bytes long and reach at most 4096 bytes back.

Arguments (JSON):
  container (default: raw)
    raw    Produce the decompressed stream as DECOMPRESSED
    darc   Decode the decompressed bytes with std/nintendo/container/darc
    sarc   Decode the decompressed bytes with std/nintendo/container/sarc

Presets:
  darc: {"container":"darc"}
  sarc: {"container":"sarc"}

Re-encoding compresses the artifacts with a greedy LZSS matcher. LZ10 has no
canonical encoding, so the rebuilt stream is a valid equivalent but is not
byte-identical to the original compressed bytes; the decompressed bytes are
reproduced exactly.
"#;

pub(crate) struct Lz10;

impl Codec for Lz10 {
  fn id(&self) -> &str {
    CODEC_ID
  }

  fn name(&self) -> &str {
    CODEC_NAME
  }

  fn description(&self) -> &str {
    CODEC_DESC
  }

  fn presets(&self) -> Vec<&str> {
    vec!["darc", "sarc"]
  }

  fn preset_args(&self, preset: &str, args: Option<&str>) -> Result<String, CodecError> {
    compression::preset_args(preset, args, self.id())
  }

  fn kind(&self) -> CodecKind {
    CodecKind::Rebuild
  }

  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    Ok(compression::can_handle(Format::Lz10, data))
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("lz10".to_owned())
  }

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    compression::decode(Format::Lz10, data, args)
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
    compression::encode(Format::Lz10, artifacts, args)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::nintendo::compression::{CompressionArgs, Container};
  use crate::codecs::{Codec, CodecHandlingConfidence, LudaredCodecArgs};

  const PLAIN: &[u8] = b"the quick brown fox jumps over the lazy dog, again and again and again";

  #[test]
  fn decodes_to_a_decompressed_artifact() {
    let compressed = super::super::lzss::compress(PLAIN, Format::Lz10).unwrap();
    let artifacts = Lz10.decode(&compressed, None).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].name, "DECOMPRESSED");
    assert_eq!(artifacts[0].data, PLAIN);
  }

  #[test]
  fn raw_encoding_round_trips() {
    let artifacts = [DecodedArtifact {
      name: "DECOMPRESSED".to_owned(),
      data: PLAIN.to_vec(),
    }];
    let encoded = Lz10.encode(Vec::new(), &artifacts, None).unwrap();
    assert_eq!(encoded[0], Format::Lz10.magic());
    assert_eq!(
      super::super::lzss::decompress(&encoded, Format::Lz10).unwrap(),
      PLAIN
    );
    assert_eq!(Lz10.decode(&encoded, None).unwrap()[0].data, PLAIN);
  }

  #[test]
  fn rejects_an_empty_or_oversized_artifact_list() {
    let error = Lz10.encode(Vec::new(), &[], None).unwrap_err();
    assert!(error.to_string().contains("found 0"), "{error}");

    let artifacts = [
      DecodedArtifact {
        name: "one".to_owned(),
        data: b"1".to_vec(),
      },
      DecodedArtifact {
        name: "two".to_owned(),
        data: b"2".to_vec(),
      },
    ];
    let error = Lz10.encode(Vec::new(), &artifacts, None).unwrap_err();
    assert!(error.to_string().contains("found 2"), "{error}");
  }

  #[test]
  fn detects_its_magic_byte() {
    assert_eq!(
      Lz10.can_handle(&[0x10, 0, 0, 0]).unwrap(),
      CodecHandlingConfidence::Likely
    );
    assert_eq!(
      Lz10.can_handle(&[0x11, 0, 0, 0]).unwrap(),
      CodecHandlingConfidence::No
    );
    assert_eq!(
      Lz10.can_handle(&[0x10, 0, 0]).unwrap(),
      CodecHandlingConfidence::No
    );
  }

  #[test]
  fn preset_arguments_can_be_overridden() {
    let args = Lz10
      .preset_args("darc", Some(r#"{"container":"raw"}"#))
      .unwrap();
    assert_eq!(
      CompressionArgs::from_args(Some(&args)).unwrap().container,
      Container::Raw
    );
  }

  #[test]
  fn rejects_an_undefined_preset() {
    let error = Lz10.preset_args("zip", None).unwrap_err();
    assert!(
      matches!(error, CodecError::UndefinedPreset(ref preset, _) if preset == "zip"),
      "{error}"
    );
  }

  #[test]
  fn rejects_unknown_arguments() {
    assert!(
      Lz10
        .decode(&[0x10, 0, 0, 0], Some(r#"{"format":"zip"}"#))
        .is_err()
    );
  }

  /// Independent streams have many valid encodings, so compare their decoded
  /// bytes after rebuilding rather than requiring byte-identical tokens.
  #[test]
  fn rebuilds_independent_streams_equivalently() {
    let fixtures = [
      (
        "nintendo_darc.lz10",
        include_bytes!(concat!(
          env!("CARGO_MANIFEST_DIR"),
          "/src/testing/fixtures/nintendo/nintendo_darc.lz10"
        ))
        .as_slice(),
      ),
      (
        "nintendo_sarc.lz10",
        include_bytes!(concat!(
          env!("CARGO_MANIFEST_DIR"),
          "/src/testing/fixtures/nintendo/nintendo_sarc.lz10"
        ))
        .as_slice(),
      ),
    ];

    for (name, fixture) in fixtures {
      let decoded = Lz10.decode(fixture, None).unwrap();
      let rebuilt = Lz10.encode(Vec::new(), &decoded, None).unwrap();
      let again = Lz10.decode(&rebuilt, None).unwrap();
      assert_eq!(again[0].data, decoded[0].data, "{name}");
    }
  }
}
