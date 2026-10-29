//! LZ11 end-to-end properties on synthetic and third-party streams.

use crate::codecs::Codec;
use crate::codecs::nintendo::compression::lz11::Lz11;
use crate::testing::codecs::e2e::{
  CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const PLAIN: &[u8] = b"abcabcabc";
const PLAIN_SHA256: &str = "76b99ab4be8521d78b19bcff7d1078aabeb477bd134f404094c92cd39f051c3e";

const DARC_PRESET: Option<&str> = Some(r#"{"container":"darc"}"#);
const SARC_PRESET: Option<&str> = Some(r#"{"container":"sarc"}"#);

pub(crate) struct Lz11E2E;

fn stream() -> Vec<u8> {
  // Three literals, then a six-byte reference three bytes back (short form).
  vec![
    0x11,
    9,
    0,
    0,
    0b0001_0000,
    b'a',
    b'b',
    b'c',
    0x50,
    0x02,
  ]
}

fn chained() -> Vec<u8> {
  let source = super::sarc_source();
  Lz11
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

impl CodecE2E for Lz11E2E {
  const CODEC: &'static dyn Codec = &Lz11;

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
        name: "compressed SARC through the sarc preset",
        input: chained(),
        args: Some(r#"{"container":"sarc"}"#),
        expected_artifacts: vec![
          ExpectedArtifact {
            name: "SARC.json",
            size: 707,
            sha256: "fb5ef82c2869159d05533ebed41d5e6ddf387309b164c5bad331d4f5b20be755",
          },
          ExpectedArtifact {
            name: "files/data/world.bin",
            size: 17,
            sha256: "2a5f6aa1f8a9c6364b9ad59d6c4b171dfd26bc66ebd7b46f4d3a21d10c9f246f",
          },
          ExpectedArtifact {
            name: "files/hello.bin",
            size: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
          },
        ],
      },
      DecodeGoldenFixture {
        name: "nlzss LZ11 over the dnasdw darctool DARC",
        input: super::DARC_LZ11_INDEPENDENT.to_vec(),
        args: None,
        expected_artifacts: vec![ExpectedArtifact {
          name: super::COMPRESSED,
          size: super::DARC_INDEPENDENT_SIZE,
          sha256: super::DARC_INDEPENDENT_SHA256,
        }],
      },
      DecodeGoldenFixture {
        name: "nlzss LZ11 over the 3DSkit SARC",
        input: super::SARC_LZ11_INDEPENDENT.to_vec(),
        args: None,
        expected_artifacts: vec![ExpectedArtifact {
          name: super::COMPRESSED,
          size: super::SARC_INDEPENDENT_SIZE,
          sha256: super::SARC_INDEPENDENT_SHA256,
        }],
      },
      DecodeGoldenFixture {
        name: "nlzss LZ11 DARC through the darc preset",
        input: super::DARC_LZ11_INDEPENDENT.to_vec(),
        args: DARC_PRESET,
        expected_artifacts: super::darc_fixture_artifacts(),
      },
      DecodeGoldenFixture {
        name: "nlzss LZ11 SARC through the sarc preset",
        input: super::SARC_LZ11_INDEPENDENT.to_vec(),
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
      expected_sha256: "638846665ff15de357fbeec02201b2d8399b100de8ed2c10a1e5e7d2dfb6b269",
    })
  }
}

codec_e2e_tests!(Lz11E2E, decode_golden, encode_golden);
