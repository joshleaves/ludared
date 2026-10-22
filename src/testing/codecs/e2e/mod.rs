//! Reusable end-to-end checks over a codec and its fixtures.
//!
//! This module defines the three property categories:
//!
//! - `bit_exact`: `encode(&[], &decode(input), args) == input` (for Rebuild
//!   codecs; for Patch codecs, the original input is passed as-is).
//! - `decode_golden`: `decode(input, args)` produces the recorded artifacts,
//!   identified by name, size and SHA-256.
//! - `encode_golden`: `encode(artifacts, args)` produces the recorded bytes,
//!   pinned to an expected SHA-256.
//!
//! A `CodecE2E` implementation declares a `CODEC` and the fixtures for the
//! properties it registers. The declarative `codec_e2e_tests!` macro generates
//! ordinary `#[test]` functions for the listed properties, and fails any
//! property that declares fixtures but provides none.

use crate::codecs::{Codec, CodecKind, DecodedArtifact};
use crate::hash::sha256_bytes as sha256_hex;

pub(crate) struct ExpectedArtifact {
  pub name: &'static str,
  pub size: usize,
  pub sha256: &'static str,
}

pub(crate) struct BitExactFixture {
  pub name: &'static str,
  pub input: Vec<u8>,
  pub args: Option<&'static str>,
}

pub(crate) struct DecodeGoldenFixture {
  pub name: &'static str,
  pub input: Vec<u8>,
  pub args: Option<&'static str>,
  pub expected_artifacts: Vec<ExpectedArtifact>,
}

pub(crate) struct EncodeGoldenFixture {
  pub name: &'static str,
  pub artifacts: Vec<DecodedArtifact>,
  pub args: Option<&'static str>,
  pub expected_sha256: &'static str,
}

/// The codec-independent part of the three end-to-end properties.
///
/// An implementation only has to name its codec and hand over the fixtures for
/// the properties it registers; the assertions themselves live in the free
/// functions below.
pub(crate) trait CodecE2E {
  const CODEC: &'static dyn Codec;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::empty()
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::empty()
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    std::iter::empty()
  }

  /// Asserts the `bit_exact` property for every fixture of `C`.
  fn assert_bit_exact() {
    let codec = Self::CODEC;
    let mut fixtures = 0usize;

    for fixture in Self::bit_exact_fixtures() {
      fixtures += 1;

      let artifacts = codec
        .decode(&fixture.input, fixture.args)
        .unwrap_or_else(|err| panic!("{}: decode failed: {err}", fixture.name));

      let original = match codec.kind() {
        CodecKind::Rebuild => Vec::new(),
        CodecKind::Patch => fixture.input.clone(),
      };
      let rebuilt = codec
        .encode(original, &artifacts, fixture.args)
        .unwrap_or_else(|err| panic!("{}: encode failed: {err}", fixture.name));

      assert!(
        rebuilt == fixture.input,
        "{}: the rebuild is not bit-exact",
        fixture.name,
      );
    }

    assert!(
      fixtures > 0,
      "{}: bit_exact was registered but provides no fixtures",
      codec.id(),
    );
  }

  /// Asserts the `decode_golden` property for every fixture of `C`.
  fn assert_decode_golden() {
    let codec = Self::CODEC;
    let mut fixtures = 0usize;

    for fixture in Self::decode_golden_fixtures() {
      fixtures += 1;

      let artifacts = codec
        .decode(&fixture.input, fixture.args)
        .unwrap_or_else(|err| panic!("{}: decode failed: {err}", fixture.name));

      let names: Vec<&str> = artifacts.iter().map(|a| a.name.as_str()).collect();
      let expected_names: Vec<&str> = fixture.expected_artifacts.iter().map(|a| a.name).collect();

      assert!(
        names == expected_names,
        "{}: the decode produced {names:?}, expected {expected_names:?}",
        fixture.name,
      );

      for (artifact, expected) in artifacts.iter().zip(&fixture.expected_artifacts) {
        assert!(
          artifact.data.len() == expected.size,
          "{}: {} is {} bytes, expected {}",
          fixture.name,
          artifact.name,
          artifact.data.len(),
          expected.size,
        );

        let digest = sha256_hex(&artifact.data);
        assert!(
          digest == expected.sha256,
          "{}: {} does not carry the recorded data: sha256 {digest}, expected {}",
          fixture.name,
          artifact.name,
          expected.sha256,
        );
      }
    }

    assert!(
      fixtures > 0,
      "{}: decode_golden was registered but provides no fixtures",
      codec.id(),
    );
  }

  /// Asserts the `encode_golden` property for every fixture of `C`.
  fn assert_encode_golden() {
    let codec = Self::CODEC;
    let mut fixtures = 0usize;

    for fixture in Self::encode_golden_fixtures() {
      fixtures += 1;

      let encoded = codec
        .encode(Vec::new(), &fixture.artifacts, fixture.args)
        .unwrap_or_else(|err| panic!("{}: encode failed: {err}", fixture.name));

      let digest = sha256_hex(&encoded);
      assert!(
        digest == fixture.expected_sha256,
        "{}: the encoded image is {} bytes with sha256 {digest}, expected {}",
        fixture.name,
        encoded.len(),
        fixture.expected_sha256,
      );
    }

    assert!(
      fixtures > 0,
      "{}: encode_golden was registered but provides no fixtures",
      codec.id(),
    );
  }
}

/// Registers the ordinary `#[test]` functions for the end-to-end properties a
/// codec names.
macro_rules! codec_e2e_tests {
  ($codec:ty $(, $property:ident)* $(,)?) => {
    $(
      codec_e2e_tests!(@one $codec, $property);
    )*
  };

  (@one $codec:ty, bit_exact) => {
    #[test]
    fn is_bit_exact() {
      <$codec as CodecE2E>::assert_bit_exact();
    }
  };

  (@one $codec:ty, decode_golden) => {
    #[test]
    fn is_decode_golden() {
      <$codec as CodecE2E>::assert_decode_golden();
    }
  };

  (@one $codec:ty, encode_golden) => {
    #[test]
    fn is_encode_golden() {
      <$codec as CodecE2E>::assert_encode_golden();
    }
  };
}

mod cdrom;
mod n3ds;
