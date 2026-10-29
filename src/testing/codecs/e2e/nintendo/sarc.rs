//! SARC end-to-end properties on a synthetic and a third-party archive.

use crate::codecs::Codec;
use crate::codecs::nintendo::container::sarc::Sarc;
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

pub(crate) struct SarcE2E;

impl CodecE2E for SarcE2E {
  const CODEC: &'static dyn Codec = &Sarc;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    [
      BitExactFixture {
        name: "synthetic SARC",
        input: super::sarc_source(),
        args: None,
      },
      BitExactFixture {
        name: "3DSkit SARC",
        input: super::SARC_INDEPENDENT.to_vec(),
        args: None,
      },
    ]
    .into_iter()
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    [
      DecodeGoldenFixture {
        name: "synthetic SARC",
        input: super::sarc_source(),
        args: None,
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
        name: "3DSkit SARC",
        input: super::SARC_INDEPENDENT.to_vec(),
        args: None,
        expected_artifacts: super::sarc_fixture_artifacts(),
      },
    ]
    .into_iter()
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    let artifacts = Sarc.decode(&super::sarc_source(), None).unwrap();
    std::iter::once(EncodeGoldenFixture {
      name: "synthetic SARC",
      artifacts,
      args: None,
      expected_sha256: "25dd3868080d04fdc2cab66c8acae418ea7694a2b617461867d70fc097ec54c0",
    })
  }
}

codec_e2e_tests!(SarcE2E, bit_exact, decode_golden, encode_golden);
