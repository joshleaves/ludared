//! E2E properties for CIA, using only deterministic synthetic bytes.

use super::synthetic::source_for;
use crate::codecs::Codec;
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const SOURCE: &str = "cia";
const SOURCE_SHA256: &str = "fc391fbfb9ac78887b9496c2eac31aba2f49136001a3e4809b660dfd933b1c77";

pub(crate) struct CiaE2E;

impl CodecE2E for CiaE2E {
  const CODEC: &'static dyn Codec = &crate::codecs::nintendo::n3ds::container::cia::N3dsCia;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "synthetic CIA",
      input: source_for(SOURCE),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "synthetic CIA",
      input: source_for(SOURCE),
      args: None,
      expected_artifacts: vec![
        ExpectedArtifact {
          name: "CIA.json",
          size: 16678,
          sha256: "fa59000d9e4a7e83da9e3ea96d58cf3cb2f98ca27d782c6e7116d5aa9dc3a2ab",
        },
        ExpectedArtifact {
          name: "certchain.bin",
          size: 2304,
          sha256: "678cf68d25cd2ffeaea2b3e6ea3c1fb920152d00399326e58ddaae6201498953",
        },
        ExpectedArtifact {
          name: "ticket.bin",
          size: 1024,
          sha256: "3cac1ce6b3157db0536a073d047d00de410176a2503f986f10444798e430fa51",
        },
        ExpectedArtifact {
          name: "tmd.bin",
          size: 2868,
          sha256: "007fb4c9ce785871dfcd64e2f0b34577d13fb5a30dc468d9cdc85da998f431b8",
        },
        ExpectedArtifact {
          name: "content0.bin",
          size: 4660,
          sha256: "659e2d221aa04c3c10f8183dd5d39b1a47bc1e7c793ad8e7f85e28b78a0c29d5",
        },
      ],
    })
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    let artifacts = Self::CODEC.decode(&source_for(SOURCE), None).unwrap();
    std::iter::once(EncodeGoldenFixture {
      name: "synthetic CIA",
      artifacts,
      args: None,
      expected_sha256: SOURCE_SHA256,
    })
  }
}

codec_e2e_tests!(CiaE2E, bit_exact, decode_golden, encode_golden);
