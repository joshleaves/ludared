//! End-to-end tests for the Nintendo 3DS BCFNT codec.

use crate::codecs::nintendo::font::bcfnt::N3dsBcfnt;
use crate::codecs::nintendo::font::bcfnt::metadata::{
  CMAP_NAME, CWDH_NAME, CmapMetadata, CmapSection, CwdhMetadata,
};
use crate::codecs::{Codec, DecodedArtifact};
use crate::testing::codecs::e2e::{
  BitExactFixture, CodecE2E, DecodeGoldenFixture, EncodeGoldenFixture, ExpectedArtifact,
};

const SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/n3ds/nintendo_3ds_bcfnt.bcfnt"
));

const ETC1_SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/n3ds/nintendo_3ds_bcfnt_etc1.bcfnt"
));

const PADDED_SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/n3ds/nintendo_3ds_bcfnt_padded.bcfnt"
));

const REBUILT_SHA256: &str = "f1c2ea0e6954c0b453a2b2c8785017e4cc2d62d08d2953d96f49acafbefffad8";

const MODIFIED_REBUILT_SHA256: &str =
  "e232100f9a6b857dad0120f507c0f346567116a97c7b351baf84a2078708ed51";

const ETC1_REBUILT_SHA256: &str =
  "5c5cd56fa0a899dd8825ccda50ded6281b4f529205657ccb795b0e67e1a6badd";

const ARTIFACTS: [(&str, usize, &str); 4] = [
  (
    "BCFNT.json",
    517,
    "b6477d3a133adbdf1210d3e3f9ec3be3de55ab349f1344802c40a50c0b242f7e",
  ),
  (
    "CWDH.json",
    368,
    "3fbaa8f80cf9eb3e9ab3cbd6fd416a37a9e4f346c888e59f37dd127be78c82b2",
  ),
  (
    "CMAP.json",
    357,
    "8bf7c08e866e7bfac6564cefe79efce242055cae232231346464b3499c1eae56",
  ),
  (
    "texture0.png",
    118,
    "b0cd9e4924c3c5a0c580514db60919160b624ee8fd63cd1abc4f897ac58df689",
  ),
];

const ETC1_ARTIFACTS: [(&str, usize, &str); 4] = [
  (
    "BCFNT.json",
    516,
    "48e343ff3584a339b0727bf3aee988446c0be0b54002d2709eb4407800daf9a9",
  ),
  (
    "CWDH.json",
    368,
    "3fbaa8f80cf9eb3e9ab3cbd6fd416a37a9e4f346c888e59f37dd127be78c82b2",
  ),
  (
    "CMAP.json",
    357,
    "8bf7c08e866e7bfac6564cefe79efce242055cae232231346464b3499c1eae56",
  ),
  (
    "texture0.png",
    118,
    "d851e4bea23058376cd552a42c83b6e7a2833e235ebceeeaf2cd93fac9a43cc2",
  ),
];

fn with_gap_and_trailer(source: &[u8]) -> Vec<u8> {
  let mut image = source.to_vec();
  let sheet_data_offset =
    u32::from_le_bytes(image[0x34 + 0x1C..0x34 + 0x20].try_into().unwrap()) as usize;
  image[0x54..sheet_data_offset].fill(0xA5);
  image.extend_from_slice(&[
    0xDE,
    0xAD,
    0xBE,
    0xEF,
  ]);
  let size = u32::try_from(image.len()).unwrap();
  image[0x0C..0x10].copy_from_slice(&size.to_le_bytes());
  image
}

fn with_tglp_trailing(source: &[u8]) -> Vec<u8> {
  let mut image = source.to_vec();
  let tglp_start = 0x34;
  let sheet_data_offset = u32::from_le_bytes(
    image[tglp_start + 0x1C..tglp_start + 0x20]
      .try_into()
      .unwrap(),
  ) as usize;
  let sheets_end = sheet_data_offset + 256;
  let tglp_end = u32::from_le_bytes(image[tglp_start + 4..tglp_start + 8].try_into().unwrap())
    as usize
    + tglp_start;
  image.splice(
    sheets_end..sheets_end,
    [
      0xCA,
      0xFE,
      0xBA,
      0xBE,
    ],
  );
  let tglp_size = u32::try_from(tglp_end + 4 - tglp_start).unwrap();
  image[tglp_start + 4..tglp_start + 8].copy_from_slice(&tglp_size.to_le_bytes());
  for offset in [
    0x14 + 0x14,
    0x14 + 0x18,
  ] {
    let old = u32::from_le_bytes(image[offset..offset + 4].try_into().unwrap());
    image[offset..offset + 4].copy_from_slice(&(old + 4).to_le_bytes());
  }
  let size = u32::try_from(image.len()).unwrap();
  image[0x0C..0x10].copy_from_slice(&size.to_le_bytes());
  image
}

fn decode() -> Vec<DecodedArtifact> {
  N3dsBcfnt
    .decode(SOURCE, None)
    .expect("the committed BCFNT fixture should decode")
}

fn modified_artifacts() -> Vec<DecodedArtifact> {
  let mut artifacts = decode();

  {
    let artifact = artifacts
      .iter_mut()
      .find(|artifact| artifact.name == CWDH_NAME)
      .expect("CWDH metadata should exist");
    let mut cwdh = CwdhMetadata::from_json(&artifact.data).expect("CWDH metadata should parse");
    cwdh.sections[0].entries[1].char_width = 42;
    artifact.data = cwdh.to_json().expect("CWDH metadata should serialize");
  }

  {
    let artifact = artifacts
      .iter_mut()
      .find(|artifact| artifact.name == CMAP_NAME)
      .expect("CMAP metadata should exist");
    let mut cmap = CmapMetadata::from_json(&artifact.data).expect("CMAP metadata should parse");
    match &mut cmap.sections[0] {
      CmapSection::Scan { entries, .. } => entries[1].glyph = 2,
      other => panic!("expected a scan CMAP, found {other:?}"),
    }
    artifact.data = cmap.to_json().expect("CMAP metadata should serialize");
  }

  artifacts
}

pub(crate) struct N3dsBcfntE2E;

impl CodecE2E for N3dsBcfntE2E {
  const CODEC: &'static dyn Codec = &N3dsBcfnt;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    [
      BitExactFixture {
        name: "nintendo_3ds_bcfnt.bcfnt",
        input: SOURCE.to_vec(),
        args: None,
      },
      BitExactFixture {
        name: "padded TGLP gap with non-zero bytes and file trailer",
        input: with_gap_and_trailer(PADDED_SOURCE),
        args: None,
      },
      BitExactFixture {
        name: "TGLP trailing bytes",
        input: with_tglp_trailing(PADDED_SOURCE),
        args: None,
      },
    ]
    .into_iter()
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    [
      DecodeGoldenFixture {
        name: "nintendo_3ds_bcfnt.bcfnt",
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
      },
      DecodeGoldenFixture {
        name: "nintendo_3ds_bcfnt_etc1.bcfnt",
        input: ETC1_SOURCE.to_vec(),
        args: None,
        expected_artifacts: ETC1_ARTIFACTS
          .iter()
          .map(|(name, size, sha256)| ExpectedArtifact {
            name,
            size: *size,
            sha256,
          })
          .collect(),
      },
    ]
    .into_iter()
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
        name: "changed width and mapping",
        artifacts: modified_artifacts(),
        args: None,
        expected_sha256: MODIFIED_REBUILT_SHA256,
      },
      EncodeGoldenFixture {
        name: "etc1 recompression",
        artifacts: N3dsBcfnt
          .decode(ETC1_SOURCE, None)
          .expect("the ETC1 fixture should decode"),
        args: None,
        expected_sha256: ETC1_REBUILT_SHA256,
      },
    ]
    .into_iter()
  }
}

codec_e2e_tests!(N3dsBcfntE2E, bit_exact, decode_golden, encode_golden);
