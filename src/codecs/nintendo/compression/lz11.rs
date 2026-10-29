//! Nintendo LZ11 (LZSS) compression codec.

use crate::codecs::nintendo::compression::{self, lzss::Format};
use crate::codecs::{
  Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact, errors::CodecError,
};

const CODEC_ID: &str = "std/nintendo/compression/lz11";
const CODEC_NAME: &str = "Nintendo LZ11 compression";
const CODEC_DESC: &str = r#"Nintendo LZ11 (LZSS) compressed stream

Decompresses an LZ11 stream into a single DECOMPRESSED artifact, or passes the
decompressed bytes to an archive codec when the 'container' argument names one.

The stream is a four-byte header (0x11 and the 24-bit little-endian
decompressed size) followed by flag bytes and literal or back-reference
tokens. References reach at most 4096 bytes back and are 3 to 16, 17 to 272,
or 273 to 65808 bytes long depending on their encoded shape.

Arguments (JSON):
  container (default: raw)
    raw    Produce the decompressed stream as DECOMPRESSED
    darc   Decode the decompressed bytes with std/nintendo/container/darc
    sarc   Decode the decompressed bytes with std/nintendo/container/sarc

Presets:
  darc: {"container":"darc"}
  sarc: {"container":"sarc"}

Re-encoding compresses the artifacts with a greedy LZSS matcher. LZ11 has no
canonical encoding, so the rebuilt stream is a valid equivalent but is not
byte-identical to the original compressed bytes; the decompressed bytes are
reproduced exactly.
"#;

pub(crate) struct Lz11;

impl Codec for Lz11 {
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
    Ok(compression::can_handle(Format::Lz11, data))
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("lz11".to_owned())
  }

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    compression::decode(Format::Lz11, data, args)
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
    compression::encode(Format::Lz11, artifacts, args)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::{Codec, CodecHandlingConfidence};

  const PLAIN: &[u8] = b"the quick brown fox jumps over the lazy dog, again and again and again";

  #[test]
  fn raw_encoding_round_trips() {
    let artifacts = [DecodedArtifact {
      name: "DECOMPRESSED".to_owned(),
      data: PLAIN.to_vec(),
    }];
    let encoded = Lz11.encode(Vec::new(), &artifacts, None).unwrap();
    assert_eq!(encoded[0], Format::Lz11.magic());
    assert_eq!(
      super::super::lzss::decompress(&encoded, Format::Lz11).unwrap(),
      PLAIN
    );
    assert_eq!(Lz11.decode(&encoded, None).unwrap()[0].data, PLAIN);
  }

  #[test]
  fn detects_its_magic_byte() {
    assert_eq!(
      Lz11.can_handle(&[0x11, 0, 0, 0]).unwrap(),
      CodecHandlingConfidence::Likely
    );
    assert_eq!(
      Lz11.can_handle(&[0x10, 0, 0, 0]).unwrap(),
      CodecHandlingConfidence::No
    );
    assert_eq!(Lz11.can_handle(&[]).unwrap(), CodecHandlingConfidence::No);
  }

  /// Independent streams have many valid encodings, so compare their decoded
  /// bytes after rebuilding rather than requiring byte-identical tokens.
  #[test]
  fn rebuilds_independent_streams_equivalently() {
    let fixtures = [
      (
        "nintendo_darc.lz11",
        include_bytes!(concat!(
          env!("CARGO_MANIFEST_DIR"),
          "/src/testing/fixtures/nintendo/nintendo_darc.lz11"
        ))
        .as_slice(),
      ),
      (
        "nintendo_sarc.lz11",
        include_bytes!(concat!(
          env!("CARGO_MANIFEST_DIR"),
          "/src/testing/fixtures/nintendo/nintendo_sarc.lz11"
        ))
        .as_slice(),
      ),
    ];

    for (name, fixture) in fixtures {
      let decoded = Lz11.decode(fixture, None).unwrap();
      let rebuilt = Lz11.encode(Vec::new(), &decoded, None).unwrap();
      let again = Lz11.decode(&rebuilt, None).unwrap();
      assert_eq!(again[0].data, decoded[0].data, "{name}");
    }
  }
}
