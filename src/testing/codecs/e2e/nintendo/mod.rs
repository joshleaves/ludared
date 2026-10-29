//! End-to-end checks for the Nintendo archive and compression codecs.
//!
//! Most fixtures are built from synthetic bytes in this module. The
//! `nintendo_*` archives and streams come from third-party tooling instead (see
//! `src/testing/fixtures/nintendo/README.md`), so they pin interoperability
//! rather than self-consistency.

use crate::codecs::Codec;
use crate::codecs::DecodedArtifact;
use crate::testing::codecs::e2e::ExpectedArtifact;

pub(crate) mod darc;
pub(crate) mod lz10;
pub(crate) mod lz11;
pub(crate) mod sarc;

pub(crate) fn artifact(name: &str, data: &[u8]) -> DecodedArtifact {
  DecodedArtifact {
    name: name.to_owned(),
    data: data.to_vec(),
  }
}

pub(crate) const DARC_METADATA: &str = "DARC.json";
pub(crate) const SARC_METADATA: &str = "SARC.json";
pub(crate) const COMPRESSED: &str = "DECOMPRESSED";

/// Archives and streams built by third-party tooling, see
/// `src/testing/fixtures/nintendo/README.md`.
pub(crate) const DARC_INDEPENDENT: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/nintendo/nintendo_darc.arc"
));
pub(crate) const SARC_INDEPENDENT: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/nintendo/nintendo_sarc.sarc"
));
pub(crate) const DARC_LZ10_INDEPENDENT: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/nintendo/nintendo_darc.lz10"
));
pub(crate) const DARC_LZ11_INDEPENDENT: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/nintendo/nintendo_darc.lz11"
));
pub(crate) const SARC_LZ10_INDEPENDENT: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/nintendo/nintendo_sarc.lz10"
));
pub(crate) const SARC_LZ11_INDEPENDENT: &[u8] = include_bytes!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/src/testing/fixtures/nintendo/nintendo_sarc.lz11"
));

pub(crate) const DARC_INDEPENDENT_SIZE: usize = 2324;
pub(crate) const DARC_INDEPENDENT_SHA256: &str =
  "e71e88d56e742ef34f0e686445fd915d3c8166eec5a172a437d335375147b0b9";
pub(crate) const SARC_INDEPENDENT_SIZE: usize = 2560;
pub(crate) const SARC_INDEPENDENT_SHA256: &str =
  "4d069e0c8bdac130d2706d8e9115cb5ba160b9c3296adc63aa638b89db19fd34";

/// Decoded artifacts of `nintendo_darc.arc`, in physical table order.
pub(crate) fn darc_fixture_artifacts() -> Vec<ExpectedArtifact> {
  vec![
    ExpectedArtifact {
      name: DARC_METADATA,
      size: 1097,
      sha256: "e04e947416daef16974c9acb1206507117a448c0bae60ddf76607e96dc3c7970",
    },
    ExpectedArtifact {
      name: "files/hi.txt",
      size: 20,
      sha256: "2b60e2f2eb8a463e852baef8c59d817fccbe59f3ebcf029bee382802686a3e64",
    },
    ExpectedArtifact {
      name: "files/data/pattern.bin",
      size: 1280,
      sha256: "892d09da5f31b759ad025a291ce85055e3e34ada6b19d726ef41a40b9ca4f3f7",
    },
    ExpectedArtifact {
      name: "files/data/nested/noise.bin",
      size: 777,
      sha256: "ca8f7586dfc69e7c8353dda6160ff2faa6d65a5684cf19a2836a32262fc383d1",
    },
  ]
}

/// Decoded artifacts of `nintendo_sarc.sarc`, in hash-table order.
pub(crate) fn sarc_fixture_artifacts() -> Vec<ExpectedArtifact> {
  vec![
    ExpectedArtifact {
      name: SARC_METADATA,
      size: 1139,
      sha256: "200008b1f2a6cc6dbc14baf6948fa754a229d257246edc082cc30f5784d748af",
    },
    ExpectedArtifact {
      name: "files/data/pattern.bin",
      size: 1280,
      sha256: "892d09da5f31b759ad025a291ce85055e3e34ada6b19d726ef41a40b9ca4f3f7",
    },
    ExpectedArtifact {
      name: "files/hi.txt",
      size: 20,
      sha256: "2b60e2f2eb8a463e852baef8c59d817fccbe59f3ebcf029bee382802686a3e64",
    },
    ExpectedArtifact {
      name: "files/data/nested/noise.bin",
      size: 777,
      sha256: "ca8f7586dfc69e7c8353dda6160ff2faa6d65a5684cf19a2836a32262fc383d1",
    },
  ]
}

pub(crate) fn darc_artifacts() -> Vec<DecodedArtifact> {
  let metadata = serde_json::json!({
    "endian": "little",
    "header_size": 0x1C,
    "version": 0x0001_0000,
    "table_offset": 0x1C,
    "data_offset": 0,
    "name_area": "",
    "data_size": 0,
    "dot_entry": false,
    "root_name_offset": 0,
    "entries": [
      {"type": "file", "parent": "", "name": "hello.bin"},
      {"type": "folder", "parent": "", "name": "data"},
      {"type": "file", "parent": "data", "name": "world.bin"}
    ]
  });
  vec![
    artifact(
      DARC_METADATA,
      &serde_json::to_vec_pretty(&metadata).unwrap(),
    ),
    artifact("files/hello.bin", b"hello"),
    artifact("files/data/world.bin", b"world world world"),
  ]
}

pub(crate) fn darc_source() -> Vec<u8> {
  crate::codecs::nintendo::container::darc::Darc
    .encode(Vec::new(), &darc_artifacts(), None)
    .unwrap()
}

pub(crate) fn sarc_artifacts() -> Vec<DecodedArtifact> {
  let metadata = serde_json::json!({
    "endian": "little",
    "header_size": 0x14,
    "version": 0x0100,
    "reserved": 0,
    "data_offset": 0,
    "hash_multiplier": 0x65,
    "sfat_header_size": 0x0C,
    "sfnt_header_size": 0x08,
    "sfnt_reserved": 0,
    "name_table": "",
    "data_size": 0,
    "nodes": [
      {"name": "hello.bin", "hash": 0, "attributes": 0x0100_0000, "data_start": 0, "data_end": 0},
      {"name": "data/world.bin", "hash": 0, "attributes": 0x0100_0000, "data_start": 0, "data_end": 0}
    ]
  });
  vec![
    artifact(
      SARC_METADATA,
      &serde_json::to_vec_pretty(&metadata).unwrap(),
    ),
    artifact("files/hello.bin", b"hello"),
    artifact("files/data/world.bin", b"world world world"),
  ]
}

pub(crate) fn sarc_source() -> Vec<u8> {
  crate::codecs::nintendo::container::sarc::Sarc
    .encode(Vec::new(), &sarc_artifacts(), None)
    .unwrap()
}
