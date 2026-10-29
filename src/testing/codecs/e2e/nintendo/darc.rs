//! DARC end-to-end properties on a synthetic and a third-party archive.

use crate::codecs::Codec;
use crate::codecs::nintendo::container::darc::Darc;
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

pub(crate) struct DarcE2E;

impl CodecE2E for DarcE2E {
  const CODEC: &'static dyn Codec = &Darc;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    [
      BitExactFixture {
        name: "synthetic DARC",
        input: super::darc_source(),
        args: None,
      },
      BitExactFixture {
        name: "dnasdw darctool DARC",
        input: super::DARC_INDEPENDENT.to_vec(),
        args: None,
      },
    ]
    .into_iter()
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    [
      DecodeGoldenFixture {
        name: "synthetic DARC",
        input: super::darc_source(),
        args: None,
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
        name: "dnasdw darctool DARC",
        input: super::DARC_INDEPENDENT.to_vec(),
        args: None,
        expected_artifacts: super::darc_fixture_artifacts(),
      },
    ]
    .into_iter()
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    let artifacts = Darc.decode(&super::darc_source(), None).unwrap();
    std::iter::once(EncodeGoldenFixture {
      name: "synthetic DARC",
      artifacts,
      args: None,
      expected_sha256: "8769c2df30843fb87ec4b7d660cf998ce03e99bd3a629fe2026dd8f486966fa0",
    })
  }
}

codec_e2e_tests!(DarcE2E, bit_exact, decode_golden, encode_golden);
