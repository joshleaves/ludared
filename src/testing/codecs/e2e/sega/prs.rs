//! Golden encoding and decoding checks through the public PRS codec contract.
//!
//! These synthetic inputs are generated below (CC0 1.0); no game data is used.
//! The candidate-order and matching goldens came from the original linked-chain
//! compressor and were unchanged by the corrected short-copy cost. The final
//! cost-regression golden was checked against that reference with the same fix.
//! Literal hex goldens were converted to SHA-256 without regenerating encodings.

use crate::codecs::Codec;
use crate::codecs::DecodedArtifact;
use crate::codecs::sega::compression::prs::PrsCompression;
use crate::testing::codecs::e2e::CodecE2E;
use crate::testing::codecs::e2e::DecodeGoldenFixture;
use crate::testing::codecs::e2e::EncodeGoldenFixture;
use crate::testing::codecs::e2e::ExpectedArtifact;

fn fixture(
  name: &'static str,
  data: Vec<u8>,
  args: &'static str,
  expected_sha256: &'static str,
) -> EncodeGoldenFixture {
  EncodeGoldenFixture {
    name,
    artifacts: vec![DecodedArtifact {
      name: "DECOMPRESSED".into(),
      data,
    }],
    args: Some(args),
    expected_sha256,
  }
}

pub(crate) struct PrsE2E;

impl CodecE2E for PrsE2E {
  const CODEC: &'static dyn Codec = &PrsCompression;

  // Fixed streams produced by prs-rs 2.0.3, with expected hashes computed
  // from the original synthetic content. See fixtures/sega/README.md.
  fn decode_golden_fixtures() -> impl Iterator<Item = DecodeGoldenFixture> {
    let fixtures: [(&str, &[u8], usize, &str); 4] = [
      (
        "empty.prs",
        include_bytes!("../../../fixtures/sega/empty.prs"),
        0,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      ),
      (
        "literals.prs",
        include_bytes!("../../../fixtures/sega/literals.prs"),
        256,
        "40aff2e9d2d8922e47afd4648e6967497158785fbd1da870e7110266bf944880",
      ),
      (
        "mixed_copies.prs",
        include_bytes!("../../../fixtures/sega/mixed_copies.prs"),
        4074,
        "e5cf92b818058ba8857d944b903b7cb90da15ffe7cbbd4518f8602be60cd5aee",
      ),
      (
        "distant_copy.prs",
        include_bytes!("../../../fixtures/sega/distant_copy.prs"),
        8447,
        "e23368c86c6fcafcb917c3dca21a69afe1578d3d0e899d4f9428b2ba5ea302c7",
      ),
    ];
    fixtures
      .into_iter()
      .map(|(name, input, size, sha256)| DecodeGoldenFixture {
        name,
        input: input.to_vec(),
        args: None,
        expected_artifacts: vec![ExpectedArtifact {
          name: "DECOMPRESSED",
          size,
          sha256,
        }],
      })
  }

  fn encode_golden_fixtures() -> impl Iterator<Item = EncodeGoldenFixture> {
    let mut fixtures = Vec::new();
    let data = b"ab1ab2ab3ab1ab2ab3ab9ab1ab2ab3";
    for (name, args, expected) in [
      (
        "candidate order chain=1",
        r#"{"chain":1}"#,
        "74db29b0c2b5ca7f3db4a72ce4793a6e60b062dea04d283f3a1dfe7d73d596b0",
      ),
      (
        "candidate order chain=256",
        r#"{"chain":256}"#,
        "fd47ebbaddef6cea80f797777bc0277975c64c434e46c5d07dcfc5de30962476",
      ),
    ] {
      fixtures.push(fixture(name, data.to_vec(), args, expected));
    }

    // Cross the maximum match length, interrupt the repetition, then reuse
    // the same distances after their cached match ends have expired.
    let data = [
      b"ab".repeat(150),
      b"X".to_vec(),
      b"ab".repeat(150),
    ]
    .concat();
    for (name, args, expected) in [
      (
        "overlapping prefixes chain=1",
        r#"{"chain":1}"#,
        "8c3990379656808807fbddd2521822c033158da2593183053365a0ee0bd0d7d6",
      ),
      (
        "overlapping prefixes chain=256",
        r#"{"chain":256}"#,
        "1c1a38b694f94328ecabc184b1c650a88c556cc145c5688c15e6997c8bae0e03",
      ),
    ] {
      fixtures.push(fixture(name, data.to_vec(), args, expected));
    }

    let mut data = Vec::new();
    for (i, length) in [
      1, 2, 3, 4, 5, 6, 31, 32, 33, 255, 256, 257, 511,
    ]
    .into_iter()
    .enumerate()
    {
      data.extend(vec![if i % 2 == 0 { 0xff } else { 0 }; length]);
      data.extend(b"shared suffix beyond the repeated-byte run".repeat(3));
      data.push(i as u8);
    }
    for (name, args, expected) in [
      (
        "run boundaries chain=1",
        r#"{"chain":1}"#,
        "76ea2bcfda7a2d4083492a14e034f199174c68d1005f800b7a7ce25bd227a3bd",
      ),
      (
        "run boundaries chain=16",
        r#"{"chain":16}"#,
        "0d9740d479ce75498065e7a167cf97099d0d1846d23890fe6bc182728b92cd95",
      ),
      (
        "run boundaries chain=256",
        r#"{"chain":256}"#,
        "b6fccd15a0e8cfe2e2e650ede21dda66231527fcdea1c4985d0f036f01df4fbd",
      ),
      (
        "run boundaries chain=4096",
        r#"{"chain":4096}"#,
        "b2ce97d96454fea85eb853cde114e489c6307ae3508a086020a357162c6dc979",
      ),
    ] {
      fixtures.push(fixture(name, data.to_vec(), args, expected));
    }

    let data = [
      vec![0; 33],
      b"suffix".repeat(20),
      vec![0; 400],
      b"suffix".repeat(20),
    ]
    .concat();
    for (name, args, expected) in [
      (
        "run filtering chain=1",
        r#"{"chain":1}"#,
        "a1ad39e5f34238af026f9542172cc89688bc1dfce3203f59c573ab03367dc842",
      ),
      (
        "run filtering chain=16",
        r#"{"chain":16}"#,
        "2eda2f0033b5b7004e8f3fa5364ccd477051ac0e4945ee5b146bc31de7842fba",
      ),
      (
        "run filtering chain=256",
        r#"{"chain":256}"#,
        "e7377295e612acb199a99e9320fd92cde874606c9b739647fdfbf33c5cf3c8f2",
      ),
      (
        "run filtering chain=4096",
        r#"{"chain":4096}"#,
        "e7377295e612acb199a99e9320fd92cde874606c9b739647fdfbf33c5cf3c8f2",
      ),
    ] {
      fixtures.push(fixture(name, data.to_vec(), args, expected));
    }

    let mut data = vec![b'A'; 48];
    data.extend(0..128);
    data.extend(vec![b'Z'; 300]);
    let data = data.repeat(40);
    for (name, args, expected) in [
      (
        "long and short matches chain=1",
        r#"{"chain":1}"#,
        "2769b9ae0303641fdd565a2680fe7406b6b8a327bd4fdbea052c071591db6d52",
      ),
      (
        "long and short matches chain=16",
        r#"{"chain":16}"#,
        "2769b9ae0303641fdd565a2680fe7406b6b8a327bd4fdbea052c071591db6d52",
      ),
      (
        "long and short matches chain=256",
        r#"{"chain":256}"#,
        "eec6d0362837ef404c72a854d40683518253e7687bec027d5c053e56c9629c17",
      ),
      (
        "long and short matches chain=4096",
        r#"{"chain":4096}"#,
        "3ce76d6baa269ba26b848330c9d44b81cffbd0205dc7b245766dedc83a225054",
      ),
    ] {
      fixtures.push(fixture(name, data.to_vec(), args, expected));
    }

    for (distance, name, expected) in [
      (
        8190,
        "window distance=8190",
        "105060dc7175038589b167bd2d9d86a22f2f5b67d69ef1392aa904768cd0697b",
      ),
      (
        8191,
        "window distance=8191",
        "4c5adfa519b1316bc7a19388bdca1d3fa55e512ae513cef6ef33ec1960cfd88f",
      ),
      (
        8192,
        "window distance=8192",
        "954169f5dd3503f15de39df1789d092a4aa53d5da3f3bdb69a38330596169e53",
      ),
    ] {
      let mut data = vec![b'A'; 40];
      data.extend(b"unique suffix".repeat(16));
      data.resize(distance, b'B');
      data.extend(vec![b'A'; 41]);
      data.extend(b"unique suffix".repeat(16));
      // One byte into the second A run, the earlier equal-length run is
      // distance + 1 bytes back; only the first case is inside the window.
      fixtures.push(fixture(name, data, r#"{"chain":256}"#, expected));
    }

    // Three short copies cost 36 bits; a 14-byte long copy plus a literal
    // costs 35. The old 10-bit short-copy score produced 38 bytes instead of 37.
    fixtures.push(fixture(
      "long copy and literal beat three short copies",
      b"\x00\x01ABCDEFGHIJKLMNX|ABCDE!FGHIJ!KLMNO!\xffABCDEFGHIJKLMNO".to_vec(),
      r#"{"chain":256}"#,
      "dd7bdfd75ea35c1853bb4a88a9e902b0fb9585c526df984de81a28051dc09ebd",
    ));
    fixtures.into_iter()
  }
}

codec_e2e_tests!(PrsE2E, decode_golden, encode_golden);
