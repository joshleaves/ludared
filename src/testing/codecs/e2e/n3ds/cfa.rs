//! E2E properties for CFA, using only deterministic synthetic bytes.

use super::synthetic::source_for;
use crate::codecs::Codec;
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const SOURCE: &str = "cfa";

pub(crate) struct CfaE2E;

impl CodecE2E for CfaE2E {
  const CODEC: &'static dyn Codec = &crate::codecs::nintendo::n3ds::container::cfa::N3dsCfa;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "synthetic CFA",
      input: source_for(SOURCE),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "synthetic CFA",
      input: source_for(SOURCE),
      args: None,
      expected_artifacts: vec![
        ExpectedArtifact {
          name: "NCCH.json",
          size: 2209,
          sha256: "6d934de2fc750b95c3d52acf24a32935bc27dac846fd94b6542d9861b1cc81cf",
        },
        ExpectedArtifact {
          name: "exefs.bin",
          size: 2048,
          sha256: "c2969b5ee654cadd9b3e65f607fb27dbcefa676572b500d3a480e2ecc63546ab",
        },
        ExpectedArtifact {
          name: "romfs.bin",
          size: 4096,
          sha256: "660ab1d17524972b902da3d925ad8bffd312a5f610b25dab1a69c58d6d7cd593",
        },
      ],
    })
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    let artifacts = Self::CODEC.decode(&source_for(SOURCE), None).unwrap();
    std::iter::once(EncodeGoldenFixture {
      name: "synthetic CFA",
      artifacts,
      args: None,
      expected_sha256: "bf45986b1caf2a517e402537b5b7edb185408086263ada668619457ae309e0ef",
    })
  }
}

codec_e2e_tests!(CfaE2E, bit_exact, decode_golden, encode_golden);
