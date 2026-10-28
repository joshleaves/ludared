//! Shared NCCH-engine E2E properties exercised through the CXI codec.

use crate::codecs::nintendo::n3ds::container::cxi::N3dsCxi;
use crate::codecs::nintendo::n3ds::container::ncch::metadata::{
  METADATA_NAME, NcchMetadata, NcchVariableSectionKind,
};
use crate::codecs::{Codec, DecodedArtifact};
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/n3ds/nintendo_3ds_ncch.ncch"
));

/// Decoding and rebuilding the fixture is an identity.
const REBUILT_SHA256: &str = "18c472c6c48fb7433ab6f880ae81d9d7f548f0ecaeace23d7bcbc1f7eed9f99d";

const REPLACED_EXEFS_SHA256: &str =
  "4dddb34f622e44d099b4a208fa8a039abcd66fb8d886172955efbc3cf0e4f96e";

const GROWN_ROMFS_SHA256: &str = "0fee854e6e4ab625234917952bc858a73c0e702c2a7a5ff1f704c6b83bb84fbd";

/// The decode of the fixture, recorded by name, size and SHA-256. The region
/// hashes come from `nintendo_3ds_ncch.py`, which computes them from the
/// documented coverage; the metadata entry pins this codec's own JSON layout.
const ARTIFACTS: [(&str, usize, &str); 7] = [
  (
    "NCCH.json",
    2649,
    "b9973933d18ae52006847f353b79d249cae68562c11caf1ad05ad19e73961786",
  ),
  (
    "exheader.bin",
    1024,
    "ec311c5d58b292d955f262c00598b922cad5b71e80bcb08d7d320f33718a685b",
  ),
  (
    "access_descriptor.bin",
    1024,
    "df08249258efba117e9975fe9813579cbc7ab8eb11476eda18fe6fee317c6150",
  ),
  (
    "logo_region.bin",
    512,
    "a9bf9704526449d219ffb8606ccc5429719e84eec008c45b2d05d58f283d111f",
  ),
  (
    "plain_region.bin",
    512,
    "a706e7adfe32ed086ec414a020f2755452a3aae6cdf48309724630930d268ca5",
  ),
  (
    "exefs.bin",
    8192,
    "ad46770b4f032d68d66c7b00bc33fb482951e5349e2b60784953e069b49e8264",
  ),
  (
    "romfs.bin",
    1024,
    "0ce5a8ec4426d832a46c977348739ab96b904753f5837a70af238d2c04e78c9b",
  ),
];

/// The same deterministic byte pattern the fixture generator uses.
fn pattern(seed: u8, size: usize) -> Vec<u8> {
  (0..size)
    .map(|offset| seed.wrapping_add((offset as u8).wrapping_mul(seed | 1)))
    .collect()
}

fn decode() -> Vec<DecodedArtifact> {
  N3dsCxi
    .decode(SOURCE, None)
    .expect("the committed NCCH fixture should decode")
}

/// The fixture rebuilt with its ExeFS replaced by different bytes of the same
/// size, so the ExeFS superblock hash is recomputed over the new content.
fn replaced_exefs() -> Vec<DecodedArtifact> {
  let mut artifacts = decode();

  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "exefs.bin")
    .expect("ExeFS should exist")
    .data = pattern(0x99, 0x2000);

  artifacts
}

/// The fixture rebuilt with its RomFS grown by four media units and the declared
/// size raised to fit, the one metadata change the case makes.
fn grown_romfs() -> Vec<DecodedArtifact> {
  let mut artifacts = decode();

  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "romfs.bin")
    .expect("RomFS should exist")
    .data
    .extend(pattern(0xA5, 0x800));

  let metadata = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == METADATA_NAME)
    .expect("metadata should exist");
  let mut parsed = NcchMetadata::from_json(&metadata.data).expect("metadata should serialize");
  parsed.content_size += 4;
  for section in &mut parsed.variable_sections {
    if section.kind == NcchVariableSectionKind::Romfs {
      section.size_available += 0x800;
    }
  }
  metadata.data = parsed.to_json().expect("metadata should serialize");

  artifacts
}

pub(crate) struct NcchEngineE2E;

impl CodecE2E for NcchEngineE2E {
  const CODEC: &'static dyn Codec = &N3dsCxi;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "nintendo_3ds_ncch.ncch",
      input: SOURCE.to_vec(),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "nintendo_3ds_ncch.ncch",
      input: SOURCE.to_vec(),
      args: None,
      expected_artifacts: ARTIFACTS
        .iter()
        .map(|(name, size, sha256)| ExpectedArtifact {
          name,
          size: *size,
          sha256,
        })
        .collect(),
    })
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    [
      EncodeGoldenFixture {
        name: "canonical",
        artifacts: decode(),
        args: None,
        expected_sha256: REBUILT_SHA256,
      },
      EncodeGoldenFixture {
        name: "replaced ExeFS",
        artifacts: replaced_exefs(),
        args: None,
        expected_sha256: REPLACED_EXEFS_SHA256,
      },
      EncodeGoldenFixture {
        name: "grown RomFS",
        artifacts: grown_romfs(),
        args: None,
        expected_sha256: GROWN_ROMFS_SHA256,
      },
    ]
    .into_iter()
  }
}

codec_e2e_tests!(NcchEngineE2E, bit_exact, decode_golden, encode_golden);
