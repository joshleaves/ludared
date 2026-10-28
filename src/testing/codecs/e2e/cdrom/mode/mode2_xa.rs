//! End-to-end checks for generic ISO9660 CD-ROM Mode 2 XA sectors.
//!
//! The checked-in BIN is a synthetic data filesystem authored with cdrtools
//! and encoded with GNU VCDImager's low-level Mode 2 XA sector routine. The
//! fixture README records the physical layout and independently built goldens.

use crate::codecs::Codec;
use crate::codecs::DecodedArtifact;
use crate::codecs::cdrom::mode::mode2_xa::Mode2Xa;
use crate::testing::codecs::e2e::BitExactFixture;
use crate::testing::codecs::e2e::CodecE2E;
use crate::testing::codecs::e2e::DecodeGoldenFixture;
use crate::testing::codecs::e2e::EncodeGoldenFixture;
use crate::testing::codecs::e2e::ExpectedArtifact;

const SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/cdrom/mode2_xa_golden.bin"
));
const COOKED_SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/cdrom/mode2_xa_cooked.iso"
));
const ISO9660_METADATA: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/cdrom/mode2_xa_iso9660.json"
));

const RAW_ARGS: &str = r#"{"filesystem":"raw"}"#;
const ISO9660_ARGS: &str = r#"{"filesystem":"iso9660"}"#;
const REBUILT_SHA256: &str = "4d8baeb16c66cfb4df7994afe5047a546351a3877ac7b428352ca9b0fb9b4646";

const RAW_ARTIFACTS: [(&str, usize, &str); 3] = [
  (
    "data.bin",
    61_440,
    "8659913b31c6fb3c3b47ba6b6a1f28e7a50a462dfc7bd7caed85f147a0198f33",
  ),
  (
    "xa_data.bin",
    280,
    "cfc047774e814268cf177e172716d3a040e68abb3ee3b1d5ae7fe17c7c7bc932",
  ),
  (
    "xa_subheaders.bin",
    120,
    "b21ccdbba33170681ab5670532b18036cc531a709527fda8e6419f35c3c71117",
  ),
];

const ISO9660_ARTIFACTS: [(&str, usize, &str); 7] = [
  (
    "XA.json",
    71,
    "fc3938de9ec230228efe615a6793ebbc73623863a54efb4c1e11c57758ef2513",
  ),
  (
    "ISO9660.json",
    4148,
    "4025e86d7968d4e96f9496b43b46c6a46c6645b14feeeb57168fbcf8aea20618",
  ),
  (
    "PLAIN.TXT",
    3000,
    "d27ce922a6279c04fa85c64fae72e67d8339d15eaaa81e4b733f6ce674e38bc2",
  ),
  (
    "XA/FORM1.BIN",
    4096,
    "6c0294b06b53f1e89f9978a127bf9ee4182239125570f700aa47f39d79a6d278",
  ),
  (
    "XA/FORM2.BIN",
    2324,
    "0a60471b6924903e2a8f0e33d9dc156997b56579c2fb4f1f287abd7dc23fd467",
  ),
  (
    "XA/FORM1.BIN.XA",
    6,
    "09ba661a81cb2e05f4cf9f66f7e49980d68e67f352be3d22d2c5630e8996bec4",
  ),
  (
    "XA/FORM2.BIN.XA",
    3,
    "d68ac288c7f961c40703dcf301d84e713fe6e65bd6ccbbadffb27dd7093f2df2",
  ),
];

fn plain_data() -> Vec<u8> {
  (0..3000).map(|index| (index * 11 + 5) as u8).collect()
}

fn form2_extra() -> Vec<u8> {
  (0..276).map(|index| (index * 19 + 11) as u8).collect()
}

fn form2_file() -> Vec<u8> {
  let mut data: Vec<u8> = (0..2048).map(|index| (index * 17 + 9) as u8).collect();
  data.extend(form2_extra());
  data
}

fn form1_file() -> Vec<u8> {
  (0..4096).map(|index| (index * 13 + 7) as u8).collect()
}

fn xa_metadata() -> Vec<u8> {
  br#"{
  "file_number": {
    "XA/FORM1.BIN": 2,
    "XA/FORM2.BIN": 3
  }
}"#
    .to_vec()
}

fn raw_xa_data() -> Vec<u8> {
  let mut data = Vec::with_capacity(280);
  data.extend_from_slice(&29u32.to_le_bytes());
  data.extend_from_slice(&form2_extra());
  data
}

fn raw_subheaders() -> Vec<u8> {
  let mut headers = vec![[0, 0, 0x08, 0]; 30];
  for index in [
    17, 19, 21, 23, 24, 26,
  ] {
    headers[index][2] = 0x89;
  }
  headers[16][2] = 0x09;
  headers[27] = [2, 3, 0x08, 0];
  headers[28] = [2, 3, 0x89, 0];
  headers[29] = [3, 1, 0xA9, 0];
  headers.into_iter().flatten().collect()
}

fn raw_artifacts() -> Vec<DecodedArtifact> {
  vec![
    DecodedArtifact {
      name: "data.bin".to_owned(),
      data: COOKED_SOURCE.to_vec(),
    },
    DecodedArtifact {
      name: "xa_data.bin".to_owned(),
      data: raw_xa_data(),
    },
    DecodedArtifact {
      name: "xa_subheaders.bin".to_owned(),
      data: raw_subheaders(),
    },
  ]
}

fn iso9660_artifacts() -> Vec<DecodedArtifact> {
  vec![
    DecodedArtifact {
      name: "XA.json".to_owned(),
      data: xa_metadata(),
    },
    DecodedArtifact {
      name: "ISO9660.json".to_owned(),
      data: ISO9660_METADATA.to_vec(),
    },
    DecodedArtifact {
      name: "PLAIN.TXT".to_owned(),
      data: plain_data(),
    },
    DecodedArtifact {
      name: "XA/FORM1.BIN".to_owned(),
      data: form1_file(),
    },
    DecodedArtifact {
      name: "XA/FORM2.BIN".to_owned(),
      data: form2_file(),
    },
    DecodedArtifact {
      name: "XA/FORM1.BIN.XA".to_owned(),
      data: vec![
        3,
        0x08,
        0,
        3,
        0x89,
        0,
      ],
    },
    DecodedArtifact {
      name: "XA/FORM2.BIN.XA".to_owned(),
      data: vec![1, 0xA9, 0],
    },
  ]
}

fn expected_artifacts(values: &[(&'static str, usize, &'static str)]) -> Vec<ExpectedArtifact> {
  values
    .iter()
    .map(|(name, size, sha256)| ExpectedArtifact {
      name,
      size: *size,
      sha256,
    })
    .collect()
}

pub(crate) struct Mode2XaE2E;

impl CodecE2E for Mode2XaE2E {
  const CODEC: &'static dyn Codec = &Mode2Xa;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    [
      RAW_ARGS,
      ISO9660_ARGS,
    ]
    .map(|args| BitExactFixture {
      name: "mode2_xa_golden.bin",
      input: SOURCE.to_vec(),
      args: Some(args),
    })
    .into_iter()
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    [
      DecodeGoldenFixture {
        name: "mode2_xa_golden.bin (raw)",
        input: SOURCE.to_vec(),
        args: Some(RAW_ARGS),
        expected_artifacts: expected_artifacts(&RAW_ARTIFACTS),
      },
      DecodeGoldenFixture {
        name: "mode2_xa_golden.bin (iso9660)",
        input: SOURCE.to_vec(),
        args: Some(ISO9660_ARGS),
        expected_artifacts: expected_artifacts(&ISO9660_ARTIFACTS),
      },
    ]
    .into_iter()
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    [
      EncodeGoldenFixture {
        name: "explicit raw data and XA sectors",
        artifacts: raw_artifacts(),
        args: Some(RAW_ARGS),
        expected_sha256: REBUILT_SHA256,
      },
      EncodeGoldenFixture {
        name: "explicit ISO9660 files and XA metadata",
        artifacts: iso9660_artifacts(),
        args: Some(ISO9660_ARGS),
        expected_sha256: REBUILT_SHA256,
      },
    ]
    .into_iter()
  }
}

codec_e2e_tests!(Mode2XaE2E, bit_exact, decode_golden, encode_golden);
