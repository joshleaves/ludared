//! LZ10 end-to-end properties on synthetic and third-party streams.

use crate::codecs::Codec;
use crate::codecs::nintendo::compression::lz10::Lz10;
use crate::testing::codecs::e2e::{
  CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const PLAIN: &[u8] = b"abcabcabc";
const PLAIN_SHA256: &str = "76b99ab4be8521d78b19bcff7d1078aabeb477bd134f404094c92cd39f051c3e";

const DARC_PRESET: Option<&str> = Some(r#"{"container":"darc"}"#);
const SARC_PRESET: Option<&str> = Some(r#"{"container":"sarc"}"#);

pub(crate) struct Lz10E2E;

fn stream() -> Vec<u8> {
  // Three literals, then a six-byte reference three bytes back.
  vec![
    0x10,
    9,
    0,
    0,
    0b0001_0000,
    b'a',
    b'b',
    b'c',
    0x30,
    0x02,
  ]
}

fn chained() -> Vec<u8> {
  let source = super::darc_source();
  Lz10
    .encode(
      Vec::new(),
      &[super::artifact(
        super::COMPRESSED,
        &source,
      )],
      None,
    )
    .unwrap()
}

impl CodecE2E for Lz10E2E {
  const CODEC: &'static dyn Codec = &Lz10;

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    [
      DecodeGoldenFixture {
        name: "literal and reference stream",
        input: stream(),
        args: None,
        expected_artifacts: vec![ExpectedArtifact {
          name: super::COMPRESSED,
          size: 9,
          sha256: PLAIN_SHA256,
        }],
      },
      DecodeGoldenFixture {
        name: "compressed DARC through the darc preset",
        input: chained(),
        args: Some(r#"{"container":"darc"}"#),
        expected_artifacts: vec![
          ExpectedArtifact {
            name: "DARC.json",
            size: 722,
            sha256: "adf25448205de2de1da5838c71c2434abe2bc340e8a5a1c03ecc6ea6789ed1fb",
          },
          ExpectedArtifact {
            name: "files/hello.bin",
            size: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
          },
          ExpectedArtifact {
            name: "files/data/world.bin",
            size: 17,
            sha256: "2a5f6aa1f8a9c6364b9ad59d6c4b171dfd26bc66ebd7b46f4d3a21d10c9f246f",
          },
        ],
      },
      DecodeGoldenFixture {
        name: "nlzss LZ10 over the dnasdw darctool DARC",
        input: super::DARC_LZ10_INDEPENDENT.to_vec(),
        args: None,
        expected_artifacts: vec![ExpectedArtifact {
          name: super::COMPRESSED,
          size: super::DARC_INDEPENDENT_SIZE,
          sha256: super::DARC_INDEPENDENT_SHA256,
        }],
      },
      DecodeGoldenFixture {
        name: "nlzss LZ10 over the 3DSkit SARC",
        input: super::SARC_LZ10_INDEPENDENT.to_vec(),
        args: None,
        expected_artifacts: vec![ExpectedArtifact {
          name: super::COMPRESSED,
          size: super::SARC_INDEPENDENT_SIZE,
          sha256: super::SARC_INDEPENDENT_SHA256,
        }],
      },
      DecodeGoldenFixture {
        name: "nlzss LZ10 DARC through the darc preset",
        input: super::DARC_LZ10_INDEPENDENT.to_vec(),
        args: DARC_PRESET,
        expected_artifacts: super::darc_fixture_artifacts(),
      },
      DecodeGoldenFixture {
        name: "nlzss LZ10 SARC through the sarc preset",
        input: super::SARC_LZ10_INDEPENDENT.to_vec(),
        args: SARC_PRESET,
        expected_artifacts: super::sarc_fixture_artifacts(),
      },
    ]
    .into_iter()
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    std::iter::once(EncodeGoldenFixture {
      name: "abcabcabc",
      artifacts: vec![super::artifact(
        super::COMPRESSED,
        PLAIN,
      )],
      args: None,
      expected_sha256: "0b272d9add485d82ce4616cf84c5147e8e468f6682ea32971483d28d3068a412",
    })
  }
}

codec_e2e_tests!(Lz10E2E, decode_golden, encode_golden);
