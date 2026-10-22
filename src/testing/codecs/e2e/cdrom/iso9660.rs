//! End-to-end tests for the ISO 9660 codec.
//!
//! The fixture is an image cdrtools mastered from the sources
//! `mkisofs_golden.sh` writes; `src/testing/fixtures/cdrom/README.md` records
//! its provenance and the independent validation the hashes below were pinned
//! against.

use crate::codecs::Codec;
use crate::codecs::DecodedArtifact;
use crate::codecs::cdrom::iso9660::Iso9660;
use crate::testing::codecs::e2e::BitExactFixture;
use crate::testing::codecs::e2e::CodecE2E;
use crate::testing::codecs::e2e::DecodeGoldenFixture;
use crate::testing::codecs::e2e::EncodeGoldenFixture;
use crate::testing::codecs::e2e::ExpectedArtifact;

const SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/cdrom/mkisofs_golden.iso"
));

/// The fixture's own bytes: an unchanged decode and rebuild reproduces the
/// image mkisofs wrote, so the expected output is the input, hashed.
const REBUILT_SHA256: &str = "3626b0e829dbc06c88d0be059336194de56b6e587e540a30369a18e36e47a3cf";

/// A rebuild with `DATA.BIN` grown from 1000 to 5000 bytes of `0xA5`, which
/// does not fit the block it held: the file moves to room past the image's end
/// and the image grows to hold it, leaving every other file at its own extent.
/// Pinned from a run of the codec after that run was checked file by file
/// against the unchanged decode.
const MODIFIED_REBUILT_SHA256: &str =
  "0560dd1505712f3ac904ea8a584592a138746014dd7cb07938b01343fd3415ff";

/// The decode's artifacts by name, size and SHA-256, in the order they sit on
/// the image. Every file's hash is the hash of the source bytes `mkisofs`
/// wrote into the fixture, computed from those sources rather than from a run
/// of this codec; `README.md` lists the same values.
const ARTIFACTS: [(&str, usize, &str); 10] = [
  (
    "ISO9660.json",
    5744,
    "6aceb2c684325d4735e8cfa92e1623377b4a9764400415df60d65e5e8870a3ac",
  ),
  (
    "DATA.BIN",
    1000,
    "1e9bc38cbf860b9ec31918b065f9b52476c549a782e0e7990bed8ce3868d2371",
  ),
  (
    "EXACT.BIN",
    2048,
    "fb8e6ddf27991852a37d557f82800795dff5362012e5a6bce0758571755fba4d",
  ),
  (
    "MIXED.TXT",
    19,
    "c610d798fde2d4f7b12c85cab951217466bcf0bef29301839dd032f09ba824ec",
  ),
  (
    "NOTHING.",
    18,
    "fda1001b1c32a3bf54a3780d58cb7c5cfc4c5220a41a79547cdebb7abc090061",
  ),
  (
    "OVER.BIN",
    2049,
    "b868791572fceb94d7f9ec8ebf246cd1ac9a2fe0538b92418c8c8018f941b998",
  ),
  (
    "DIR_B/DUP.BIN",
    12,
    "83260694f4619a55ad1e5e2130f4f36b16c7ae0b22c70706f22a253208876217",
  ),
  (
    "DIR_A/DUP.BIN",
    11,
    "290be41511ae958c53ba7009d34b9666bf7ffa72198e73dfe0d390e9117a1c7c",
  ),
  (
    "DIR_A/INNER.BIN",
    34,
    "a3927f0d1e452d3ebfc186a4fe7d106d10f6d300eb3efec16372c22b09072c38",
  ),
  (
    "EMPTY.BIN",
    0,
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  ),
];

fn decode() -> Vec<DecodedArtifact> {
  Iso9660
    .decode(SOURCE, None)
    .expect("the committed ISO 9660 fixture should decode")
}

fn grown_artifacts() -> Vec<DecodedArtifact> {
  let mut artifacts = decode();

  let data = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "DATA.BIN")
    .expect("the fixture should hold DATA.BIN");

  data.data = vec![0xA5; 5000];

  artifacts
}

pub(crate) struct Iso9660E2E;

impl CodecE2E for Iso9660E2E {
  const CODEC: &'static dyn Codec = &Iso9660;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "mkisofs_golden.iso",
      input: SOURCE.to_vec(),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "mkisofs_golden.iso",
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
        name: "grown DATA.BIN",
        artifacts: grown_artifacts(),
        args: None,
        expected_sha256: MODIFIED_REBUILT_SHA256,
      },
    ]
    .into_iter()
  }
}

codec_e2e_tests!(Iso9660E2E, bit_exact, decode_golden, encode_golden);
