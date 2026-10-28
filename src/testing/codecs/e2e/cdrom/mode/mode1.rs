//! End-to-end tests for the CD-ROM Mode 1 codec.
//!
//! The input is a small synthetic four-sector image. Its user data pattern is
//! also pinned in `src/codecs/cdrom/mode1.rs`'s unit test.

use crate::codecs::Codec;
use crate::codecs::DecodedArtifact;
use crate::codecs::cdrom::mode::mode1::Mode1;
use crate::testing::codecs::e2e::BitExactFixture;
use crate::testing::codecs::e2e::CodecE2E;
use crate::testing::codecs::e2e::DecodeGoldenFixture;
use crate::testing::codecs::e2e::EncodeGoldenFixture;
use crate::testing::codecs::e2e::ExpectedArtifact;

const SOURCE: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/cdrom/mode1_golden.bin"
));

const USER_DATA_SHA256: &str = "4bf9580f28aab520cc05fd341d4147c9abc7c9b95d7c1cc5dfdd7ce54e85ee6b";
const REBUILT_SHA256: &str = "fb1fbb777a57ad1c890c19b513589bc5708726734cb0eb3fc68f1a9ed1e6f7ad";

fn user_data() -> Vec<u8> {
  let mut data = vec![0u8; 2048 * 4];
  data[2048..4096].fill(0xFF);

  for (index, byte) in data[4096..].iter_mut().enumerate() {
    *byte = (index as u8).wrapping_mul(37).wrapping_add(13);
  }

  data
}

fn artifacts() -> Vec<DecodedArtifact> {
  vec![DecodedArtifact {
    name: "data.bin".to_owned(),
    data: user_data(),
  }]
}

pub(crate) struct Mode1E2E;

impl CodecE2E for Mode1E2E {
  const CODEC: &'static dyn Codec = &Mode1;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture {
      name: "mode1_golden.bin",
      input: SOURCE.to_vec(),
      args: None,
    })
  }

  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    std::iter::once(DecodeGoldenFixture {
      name: "mode1_golden.bin",
      input: SOURCE.to_vec(),
      args: None,
      expected_artifacts: vec![ExpectedArtifact {
        name: "data.bin",
        size: 8192,
        sha256: USER_DATA_SHA256,
      }],
    })
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    std::iter::once(EncodeGoldenFixture {
      name: "four synthetic user sectors",
      artifacts: artifacts(),
      args: None,
      expected_sha256: REBUILT_SHA256,
    })
  }
}

codec_e2e_tests!(Mode1E2E, bit_exact, decode_golden, encode_golden);
