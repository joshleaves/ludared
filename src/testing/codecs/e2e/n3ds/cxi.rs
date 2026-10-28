//! E2E properties for CXI, using only deterministic synthetic bytes.

use super::synthetic::source_for;
use crate::codecs::Codec;
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const SOURCE: &str = "cxi";

pub(crate) struct CxiE2E;

impl CodecE2E for CxiE2E {
  const CODEC: &'static dyn Codec = &crate::codecs::nintendo::n3ds::container::cxi::N3dsCxi;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "synthetic CXI",
      input: source_for(SOURCE),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "synthetic CXI",
      input: source_for(SOURCE),
      args: None,
      expected_artifacts: vec![
        ExpectedArtifact {
          name: "NCCH.json",
          size: 2812,
          sha256: "c67f474c680591e8795765c1378f971eddfa6c93a9a83ef25fbad807a67992a1",
        },
        ExpectedArtifact {
          name: "exheader.bin",
          size: 1024,
          sha256: "5f70bf18a086007016e948b04aed3b82103a36bea41755b6cddfaf10ace3c6ef",
        },
        ExpectedArtifact {
          name: "access_descriptor.bin",
          size: 1024,
          sha256: "5f70bf18a086007016e948b04aed3b82103a36bea41755b6cddfaf10ace3c6ef",
        },
        ExpectedArtifact {
          name: "plain_region.bin",
          size: 512,
          sha256: "3775305e808900fec0663ed8dcf409ead8633117860372fe6afd3132796ebc72",
        },
        ExpectedArtifact {
          name: "logo_region.bin",
          size: 8192,
          sha256: "19f614eb542b2e36900787167749a215c8125ee52edd42d28bef97103b073c0d",
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
      name: "synthetic CXI",
      artifacts,
      args: None,
      expected_sha256: "f951ce656922f5f3fd9f4f8f6d53e54621af54c8da39ee18eb4fb7d42156b077",
    })
  }
}

codec_e2e_tests!(CxiE2E, bit_exact, decode_golden, encode_golden);
