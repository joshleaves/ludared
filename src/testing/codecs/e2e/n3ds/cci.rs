//! End-to-end tests for the Nintendo 3DS CCI codec.

use crate::codecs::nintendo::n3ds::container::cci::N3dsCci;
use crate::codecs::nintendo::n3ds::container::cci::metadata::{
  METADATA_NAME, NcsdMetadata, partition_name,
};
use crate::codecs::{Codec, DecodedArtifact};
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/n3ds/nintendo_3ds_cci.3ds"
));

const REBUILT_SHA256: &str = "543a0b2fba6949d5941261a289742113de86bee83d9a7c13c9816a4ea28d3d5d";

const MODIFIED_REBUILT_SHA256: &str =
  "00c022a35eca1400bc02837e14f11f2a8ac141b8a0d47791791104f209c07dc5";

const ARTIFACTS: [(&str, usize, &str); 5] = [
  (
    "NCSD.json",
    1467,
    "fdcb174cb0f729e28c3da7b55db9ea8c2e615df532dbb04d61d4a782eb6fa9b5",
  ),
  (
    "cardinfo.bin",
    0x3E00,
    "cbc22f891cfbb9bbc70de30ce89b0d77e2a71d5343f93b7e51756f5dd9456970",
  ),
  (
    "partition0.ncch",
    0x2000,
    "c288c7beda83fa7b1b200304a8e4c1d24720255542653dc1f044d18f8858b888",
  ),
  (
    "partition1.ncch",
    0x4000,
    "d787991206e11e6fce6d9d3b8d72ce33f729ff6a4933e432ef90674e1414b0d0",
  ),
  (
    "trailer.bin",
    0x2000,
    "459ff52e3cd5e24d71acfa09fc52384415f63424f1b73b84e65e2e2c812ea218",
  ),
];

fn decode() -> Vec<DecodedArtifact> {
  N3dsCci
    .decode(SOURCE, None)
    .expect("the committed CCI fixture should decode")
}

fn grown_artifacts() -> Vec<DecodedArtifact> {
  let mut artifacts = decode();

  let partition = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == partition_name(0))
    .expect("partition 0 should exist");

  partition.data.extend(std::iter::repeat_n(0xA5, 0x800));

  let metadata_artifact = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == METADATA_NAME)
    .expect("NCSD metadata should exist");

  let mut metadata =
    NcsdMetadata::from_json(&metadata_artifact.data).expect("NCSD metadata should parse");

  metadata.image_size = 0x64;

  metadata_artifact.data = metadata.to_json().expect("NCSD metadata should serialize");

  artifacts
}

pub(crate) struct N3dsCciE2E;

impl CodecE2E for N3dsCciE2E {
  const CODEC: &'static dyn Codec = &N3dsCci;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "nintendo_3ds_cci.3ds",
      input: SOURCE.to_vec(),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "nintendo_3ds_cci.3ds",
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
        name: "grown partition 0",
        artifacts: grown_artifacts(),
        args: None,
        expected_sha256: MODIFIED_REBUILT_SHA256,
      },
    ]
    .into_iter()
  }
}

codec_e2e_tests!(N3dsCciE2E, bit_exact, decode_golden, encode_golden);
