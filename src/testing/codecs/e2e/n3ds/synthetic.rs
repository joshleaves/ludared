//! Deterministic, synthetic-only source images for the 3DS codec E2E tests.
//!
//! These builders use no commercial or externally sourced data.

use crate::codecs::common::hash::sha256;
use crate::codecs::nintendo::n3ds::container::ncch::testkit::build as build_ncch;
use crate::codecs::nintendo::n3ds::internal::romfs::N3dsRomFs;
use crate::codecs::{Codec, DecodedArtifact};

const CIA_HEADER_SIZE: usize = 0x2020;
const SECTION_ALIGN: usize = 0x40;
const TMD_CONTENT_COUNT_OFFSET: usize = 0x9E;
const TMD_CHUNK_RECORDS_OFFSET: usize = 0x9C4;
const TMD_CHUNK_RECORD_SIZE: usize = 0x30;
const TMD_SIGNATURE_SIZE: usize = 0x140;

pub(crate) fn cxi_source() -> Vec<u8> {
  build_ncch(
    0x400,
    &[
      ("plain.bin", 0x10, 1),
      ("logo.bin", 0x12, 0x10),
      ("exefs.bin", 0x22, 4),
      ("romfs.bin", 0x26, 8),
    ],
  )
}

pub(crate) fn cfa_source() -> Vec<u8> {
  build_ncch(
    0,
    &[
      ("exefs.bin", 0x2, 4),
      ("romfs.bin", 0x6, 8),
    ],
  )
}

fn exefs_name(name: &str) -> [u8; 8] {
  let mut result = [0; 8];
  result[..name.len()].copy_from_slice(name.as_bytes());
  result
}

pub(crate) fn exefs_source() -> Vec<u8> {
  const HEADER_SIZE: usize = 0x200;
  const FILE_HEADER_SIZE: usize = 0x10;
  const HASHES_OFFSET: usize = 0xC0;
  const HASH_SIZE: usize = 0x20;
  let files: [(&str, usize, u8); 2] = [
    ("icon", 0x80, 0x11),
    (".code", 0x40, 0x42),
  ];
  let mut header = vec![0; HEADER_SIZE];
  let mut body = Vec::new();
  for (index, (name, size, seed)) in files.iter().enumerate() {
    let offset = body.len() as u32;
    let bytes: Vec<u8> = (0..*size).map(|i| seed.wrapping_add(i as u8)).collect();
    let base = index * FILE_HEADER_SIZE;
    header[base..base + 8].copy_from_slice(&exefs_name(name));
    header[base + 8..base + 12].copy_from_slice(&offset.to_le_bytes());
    header[base + 12..base + 16].copy_from_slice(&(*size as u32).to_le_bytes());
    let hash_offset = HASHES_OFFSET + (9 - index) * HASH_SIZE;
    header[hash_offset..hash_offset + HASH_SIZE].copy_from_slice(&sha256(&bytes));
    body.extend_from_slice(&bytes);
    body.resize(body.len().next_multiple_of(0x200), 0);
  }
  header.extend_from_slice(&body);
  header
}

fn romfs_artifacts() -> Vec<DecodedArtifact> {
  let metadata = serde_json::json!({
    "magic_id": 0x10000,
    "master_hash_size": 0,
    "optional_info_size": 0x5C,
    "reserved_tail": "00000000",
    "levels": [
      {"logical_offset": 0, "hash_data_size": 0, "block_size_log2": 12, "reserved": 0},
      {"logical_offset": 0, "hash_data_size": 0, "block_size_log2": 12, "reserved": 0},
      {"logical_offset": 0, "hash_data_size": 0, "block_size_log2": 12, "reserved": 0}
    ],
    "dir_hash_buckets": 3,
    "file_hash_buckets": 3,
    "directories": [
      {"parent": "", "name": "", "name_raw": null},
      {"parent": "", "name": "data", "name_raw": null}
    ],
    "files": [
      {"parent": "", "name": "hello.bin", "name_raw": null, "data_offset": 0, "size": 8},
      {"parent": "data", "name": "empty", "name_raw": null, "data_offset": 0, "size": 0}
    ]
  });
  vec![
    DecodedArtifact {
      name: "RomFS.json".to_owned(),
      data: serde_json::to_vec_pretty(&metadata).unwrap(),
    },
    DecodedArtifact {
      name: "files/hello.bin".to_owned(),
      data: b"syntheti".to_vec(),
    },
    DecodedArtifact {
      name: "files/data/empty".to_owned(),
      data: Vec::new(),
    },
  ]
}

pub(crate) fn romfs_source() -> Vec<u8> {
  N3dsRomFs
    .encode(Vec::new(), &romfs_artifacts(), None)
    .unwrap()
}

fn build_tmd(content: &[u8]) -> Vec<u8> {
  let mut tmd = vec![0; TMD_SIGNATURE_SIZE + TMD_CHUNK_RECORDS_OFFSET + TMD_CHUNK_RECORD_SIZE];
  tmd[..4].copy_from_slice(&0x01_0004u32.to_be_bytes());
  tmd[TMD_SIGNATURE_SIZE + TMD_CONTENT_COUNT_OFFSET
    ..TMD_SIGNATURE_SIZE + TMD_CONTENT_COUNT_OFFSET + 2]
    .copy_from_slice(&1u16.to_be_bytes());
  let record = TMD_SIGNATURE_SIZE + TMD_CHUNK_RECORDS_OFFSET;
  tmd[record..record + 4].copy_from_slice(&7u32.to_be_bytes());
  tmd[record + 4..record + 6].copy_from_slice(&7u16.to_be_bytes());
  tmd[record + 6..record + 8].copy_from_slice(&1u16.to_be_bytes());
  tmd[record + 8..record + 0x10].copy_from_slice(&(content.len() as u64).to_be_bytes());
  tmd[record + 0x10..record + 0x30].copy_from_slice(&sha256(content));
  tmd
}

pub(crate) fn cia_source() -> Vec<u8> {
  let certchain = vec![0xC1; 0x900];
  let ticket = vec![0x71; 0x400];
  let content: Vec<u8> = (0..0x1234).map(|i| (i as u8).wrapping_add(1)).collect();
  let tmd = build_tmd(&content);
  let mut image = vec![0; CIA_HEADER_SIZE];
  image[..4].copy_from_slice(&(CIA_HEADER_SIZE as u32).to_le_bytes());
  image[6..8].copy_from_slice(&0x20u16.to_le_bytes());
  image[8..12].copy_from_slice(&(certchain.len() as u32).to_le_bytes());
  image[12..16].copy_from_slice(&(ticket.len() as u32).to_le_bytes());
  image[16..20].copy_from_slice(&(tmd.len() as u32).to_le_bytes());
  image[0x18..0x20].copy_from_slice(&(0x1240u64).to_le_bytes());
  image[0x20] |= 0x01;
  image.resize(image.len().next_multiple_of(SECTION_ALIGN), 0);
  for section in [
    &certchain,
    &ticket,
    &tmd,
  ] {
    image.extend_from_slice(section);
    image.resize(image.len().next_multiple_of(SECTION_ALIGN), 0);
  }
  image.extend_from_slice(&content);
  image.resize(image.len().next_multiple_of(SECTION_ALIGN), 0);
  image
}

pub(crate) fn source_for(codec: &str) -> Vec<u8> {
  match codec {
    "cia" => cia_source(),
    "cxi" => cxi_source(),
    "cfa" => cfa_source(),
    "exefs" => exefs_source(),
    "romfs" => romfs_source(),
    _ => unreachable!("unknown synthetic 3DS fixture {codec}"),
  }
}
