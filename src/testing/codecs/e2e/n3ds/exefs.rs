//! E2E properties for ExeFS, using only deterministic synthetic bytes.

use super::synthetic::source_for;
use crate::codecs::Codec;
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const SOURCE: &str = "exefs";
const SOURCE_SHA256: &str = "c33a88ba78c7e68f6bfbca02f7c62593f2d99ef2c074e02def2cbfb81cc70ac7";

pub(crate) struct ExefsE2E;

impl CodecE2E for ExefsE2E {
  const CODEC: &'static dyn Codec = &crate::codecs::nintendo::n3ds::internal::exefs::N3dsExeFs;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "synthetic ExeFS",
      input: source_for(SOURCE),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "synthetic ExeFS",
      input: source_for(SOURCE),
      args: None,
      expected_artifacts: vec![
        ExpectedArtifact {
          name: "ExeFS.json",
          size: 235,
          sha256: "811193345b9cbc996d877b04b8bdd35a8bd33a2d03ac2d5a59f9b4e0cecfc8bb",
        },
        ExpectedArtifact {
          name: "icon",
          size: 128,
          sha256: "e566cf579ecfa62245d442106a666fe2a32490572bcfaeaa57b935488be00f7b",
        },
        ExpectedArtifact {
          name: ".code",
          size: 64,
          sha256: "e5b8fe784a635157e4c353571d1dac9a0f01a38ef7d1fa867fe26149105b8fdb",
        },
      ],
    })
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    let artifacts = Self::CODEC.decode(&source_for(SOURCE), None).unwrap();
    std::iter::once(EncodeGoldenFixture {
      name: "synthetic ExeFS",
      artifacts,
      args: None,
      expected_sha256: SOURCE_SHA256,
    })
  }
}

codec_e2e_tests!(ExefsE2E, bit_exact, decode_golden, encode_golden);
