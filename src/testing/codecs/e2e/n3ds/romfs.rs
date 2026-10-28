//! E2E properties for RomFS, using only deterministic synthetic bytes.

use super::synthetic::source_for;
use crate::codecs::Codec;
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const SOURCE: &str = "romfs";
const SOURCE_SHA256: &str = "373868a66f4c81cfd6379ce15a8e93e54e7d0dd3b1a169625514261b2d46fce3";

pub(crate) struct RomfsE2E;

impl CodecE2E for RomfsE2E {
  const CODEC: &'static dyn Codec = &crate::codecs::nintendo::n3ds::internal::romfs::N3dsRomFs;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "synthetic RomFS",
      input: source_for(SOURCE),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "synthetic RomFS",
      input: source_for(SOURCE),
      args: None,
      expected_artifacts: vec![
        ExpectedArtifact {
          name: "RomFS.json",
          size: 879,
          sha256: "156e9a5cd9a38005322a840bd123c30978a4c844ba73acb92eaee2b3b74ab133",
        },
        ExpectedArtifact {
          name: "files/hello.bin",
          size: 8,
          sha256: "65c3bb15cd47fb62e4a29d549d7ccb7969c4bc80eb0ac42b7368914e8ec55061",
        },
        ExpectedArtifact {
          name: "files/data/empty",
          size: 0,
          sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        },
      ],
    })
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    let artifacts = Self::CODEC.decode(&source_for(SOURCE), None).unwrap();
    std::iter::once(EncodeGoldenFixture {
      name: "synthetic RomFS",
      artifacts,
      args: None,
      expected_sha256: SOURCE_SHA256,
    })
  }
}

codec_e2e_tests!(RomfsE2E, bit_exact, decode_golden, encode_golden);
