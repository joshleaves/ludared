//! End-to-end tests for the UDF codec.

use crate::codecs::Codec;
use crate::codecs::DecodedArtifact;
use crate::codecs::cdrom::filesystem::udf::Udf;
use crate::testing::codecs::e2e::BitExactFixture;
use crate::testing::codecs::e2e::CodecE2E;
use crate::testing::codecs::e2e::DecodeGoldenFixture;
use crate::testing::codecs::e2e::EncodeGoldenFixture;
use crate::testing::codecs::e2e::ExpectedArtifact;

const SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/cdrom/udf_golden.iso"
));

const REBUILT_SHA256: &str = "2f5e6c4d7770f66be4e6967ce1d755d179071cae26776acb064ea2fec74f2d5d";
const GROWN_REBUILT_SHA256: &str =
  "4975ba8d610147cf7e9fc6fa36a0b31fec862d72eb7aaa13b3ab241575ebfe92";

const ARTIFACTS: [(&str, usize, &str); 3] = [
  (
    "UDF.json",
    369593,
    "7928bf917cd7dac153d500a03b7f49e8c5cba5b92e5921de89f7d14c98470895",
  ),
  (
    "DATA.BIN",
    1000,
    "0e57164c5bec5951ce2e9f0ad8c9c3637d50e818fa0a6403ffdb85a886f2c6e4",
  ),
  (
    "DIR_A/NOTE.TXT",
    30,
    "d1927e49f19cd9432b8e8ca2df0cfb1cc88966c07647d0e3d16df3c2abdd3b3a",
  ),
];

fn decode() -> Vec<DecodedArtifact> {
  Udf
    .decode(SOURCE, None)
    .expect("the committed UDF fixture should decode")
}

fn grown_artifacts() -> Vec<DecodedArtifact> {
  let mut artifacts = decode();
  let data = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "DATA.BIN")
    .expect("the fixture should hold DATA.BIN");
  data.data = vec![0xA5; 1200];
  artifacts
}

pub(crate) struct UdfE2E;

impl CodecE2E for UdfE2E {
  const CODEC: &'static dyn Codec = &Udf;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "udf_golden.iso",
      input: SOURCE.to_vec(),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "udf_golden.iso",
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
        name: "canonical UDF bridge",
        artifacts: decode(),
        args: None,
        expected_sha256: REBUILT_SHA256,
      },
      EncodeGoldenFixture {
        name: "grown DATA.BIN within its reserved block",
        artifacts: grown_artifacts(),
        args: None,
        expected_sha256: GROWN_REBUILT_SHA256,
      },
    ]
    .into_iter()
  }
}

codec_e2e_tests!(UdfE2E, bit_exact, decode_golden, encode_golden);
