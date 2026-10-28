use super::*;
use crate::codecs::common::hash::sha256;
use crate::codecs::{Codec, CodecHandlingConfidence, DecodedArtifact};

const SIG_SIZE: usize = 0x140; // RSA-2048 SHA-256

fn build_tmd(contents: &[(u32, u16, &[u8])]) -> Vec<u8> {
  let mut tmd =
    vec![0u8; SIG_SIZE + TMD_CHUNK_RECORDS_OFFSET + contents.len() * TMD_CHUNK_RECORD_SIZE];
  tmd[0..4].copy_from_slice(&0x01_0004u32.to_be_bytes());
  tmd[0x40..0x48].copy_from_slice(b"Root-CA\0");
  tmd[SIG_SIZE + TMD_CONTENT_COUNT_OFFSET..SIG_SIZE + TMD_CONTENT_COUNT_OFFSET + 2]
    .copy_from_slice(&(contents.len() as u16).to_be_bytes());
  for (index, (id, content_index, data)) in contents.iter().enumerate() {
    let base = SIG_SIZE + TMD_CHUNK_RECORDS_OFFSET + index * TMD_CHUNK_RECORD_SIZE;
    tmd[base..base + 4].copy_from_slice(&id.to_be_bytes());
    tmd[base + 4..base + 6].copy_from_slice(&content_index.to_be_bytes());
    tmd[base + 6..base + 8].copy_from_slice(&1u16.to_be_bytes());
    tmd[base + 8..base + 0x10].copy_from_slice(&(data.len() as u64).to_be_bytes());
    let digest = sha256(data);
    tmd[base + 0x10..base + 0x30].copy_from_slice(&digest);
  }

  if !contents.is_empty() {
    let info_start = SIG_SIZE + TMD_CONTENT_INFO_OFFSET;
    tmd[info_start + 2..info_start + 4].copy_from_slice(&(contents.len() as u16).to_be_bytes());
    let chunk_start = SIG_SIZE + TMD_CHUNK_RECORDS_OFFSET;
    let chunk_end = chunk_start + contents.len() * TMD_CHUNK_RECORD_SIZE;
    let info_hash = sha256(&tmd[chunk_start..chunk_end]);
    tmd[info_start + 4..info_start + 0x24].copy_from_slice(&info_hash);
    let info_end = info_start + TMD_CONTENT_INFO_RECORD_COUNT * TMD_CONTENT_INFO_RECORD_SIZE;
    let header_hash = sha256(&tmd[info_start..info_end]);
    tmd[SIG_SIZE + TMD_CONTENT_INFO_HASH_OFFSET..SIG_SIZE + TMD_CONTENT_INFO_HASH_OFFSET + 0x20]
      .copy_from_slice(&header_hash);
  }
  tmd
}

fn content_bytes(seed: u8, size: usize) -> Vec<u8> {
  (0..size).map(|i| seed.wrapping_add(i as u8)).collect()
}

struct CiaBuilder {
  cert: Vec<u8>,
  ticket: Vec<u8>,
  contents: Vec<(u32, u16, Vec<u8>)>,
  meta: Vec<u8>,
}

fn builder() -> CiaBuilder {
  CiaBuilder {
    cert: vec![0xC1; 0x900],
    ticket: vec![0x71; 0x400],
    contents: vec![(7, 7, content_bytes(1, 0x1234))],
    meta: Vec::new(),
  }
}

impl CiaBuilder {
  fn meta(mut self, bytes: Vec<u8>) -> Self {
    self.meta = bytes;
    self
  }

  fn build(&self) -> Vec<u8> {
    let tmd = build_tmd(
      &self
        .contents
        .iter()
        .map(|(id, content_index, data)| (*id, *content_index, data.as_slice()))
        .collect::<Vec<_>>(),
    );
    let mut image = vec![0u8; HEADER_MIN_SIZE];
    image[0x00..0x04].copy_from_slice(&(HEADER_MIN_SIZE as u32).to_le_bytes());
    image[0x06..0x08].copy_from_slice(&0x20u16.to_le_bytes());
    image[0x08..0x0C].copy_from_slice(&(self.cert.len() as u32).to_le_bytes());
    image[0x0C..0x10].copy_from_slice(&(self.ticket.len() as u32).to_le_bytes());
    image[0x10..0x14].copy_from_slice(&(tmd.len() as u32).to_le_bytes());
    image[0x14..0x18].copy_from_slice(&(self.meta.len() as u32).to_le_bytes());
    image[0x42..0x4A].copy_from_slice(b"CONTENT!");

    let content_size: u64 = self
      .contents
      .iter()
      .map(|(_, _, data)| (data.len() as u64).next_multiple_of(SECTION_ALIGN))
      .sum::<u64>();
    image[0x18..0x20].copy_from_slice(&content_size.to_le_bytes());
    for (_, index, _) in &self.contents {
      let byte = CONTENT_INDEX_OFFSET + usize::from(*index) / 8;
      let bit = 7 - usize::from(*index) % 8;
      image[byte] |= 1 << bit;
    }

    let aligned = image.len().next_multiple_of(SECTION_ALIGN as usize);
    image.resize(aligned, 0);
    for section in [
      &self.cert,
      &self.ticket,
      &tmd,
    ] {
      image.extend_from_slice(section);
      let aligned = image.len().next_multiple_of(SECTION_ALIGN as usize);
      image.resize(aligned, 0);
    }
    for (_, _, data) in &self.contents {
      image.extend_from_slice(data);
      let aligned = image.len().next_multiple_of(SECTION_ALIGN as usize);
      image.resize(aligned, 0);
    }
    image.extend_from_slice(&self.meta);
    image
  }
}

fn decode(image: &[u8]) -> Vec<DecodedArtifact> {
  N3dsCia.decode(image, None).unwrap()
}

fn round_trip(image: &[u8]) -> Vec<u8> {
  N3dsCia.encode(Vec::new(), &decode(image), None).unwrap()
}

#[test]
fn detects_cia_packages() {
  assert_eq!(
    N3dsCia.can_handle(&builder().build()).unwrap(),
    CodecHandlingConfidence::Certain
  );
}

#[test]
fn rejects_foreign_data() {
  for image in [
    Vec::new(),
    vec![0; HEADER_MIN_SIZE],
    vec![0xFF; 0x4000],
  ] {
    assert_eq!(
      N3dsCia.can_handle(&image).unwrap(),
      CodecHandlingConfidence::No
    );
  }
}

#[test]
fn rejects_truncated_cia_padding_without_panicking() {
  let mut image = builder().build();
  // The section data is present, but the zero-padding to its next 0x40-byte
  // boundary is not. A decoder must not slice past the supplied archive.
  image.pop();

  assert_eq!(
    N3dsCia.can_handle(&image).unwrap(),
    CodecHandlingConfidence::No
  );
  assert!(N3dsCia.decode(&image, None).is_err());
}

#[test]
fn rejects_archive_header_sizes_past_the_input() {
  let mut image = builder().build();
  image[0..4].copy_from_slice(&u32::MAX.to_le_bytes());

  assert_eq!(
    N3dsCia.can_handle(&image).unwrap(),
    CodecHandlingConfidence::No
  );
  assert!(N3dsCia.decode(&image, None).is_err());
}

#[test]
fn rejects_duplicate_tmd_content_indices() {
  let mut package = builder();
  let second = content_bytes(0x31, 0x20);
  package.contents.push((8, 7, second));
  let image = package.build();

  assert_eq!(
    N3dsCia.can_handle(&image).unwrap(),
    CodecHandlingConfidence::No
  );
  assert!(N3dsCia.decode(&image, None).is_err());
}

#[test]
fn round_trips_packages() {
  for image in [
    builder().build(),
    builder().meta(vec![0x99; 0x400]).build(),
  ] {
    assert_eq!(round_trip(&image), image);
  }
}

#[test]
fn decodes_sections_and_contents() {
  let image = builder().build();
  let artifacts = decode(&image);
  let names: Vec<&str> = artifacts.iter().map(|a| a.name.as_str()).collect();
  assert_eq!(
    names,
    [
      METADATA_NAME,
      CERTCHAIN_NAME,
      TICKET_NAME,
      TMD_NAME,
      "content0.bin"
    ]
  );
  assert_eq!(artifacts[4].data, content_bytes(1, 0x1234));
}

#[test]
fn round_trips_content_region_with_no_active_contents() {
  let mut image = builder().build();
  image[CONTENT_INDEX_OFFSET..CONTENT_INDEX_OFFSET + CONTENT_INDEX_SIZE].fill(0);
  let content_size = 0x1240;
  let content_end = image.len();
  let content_start = content_end - content_size;
  image[content_start..content_end].fill(0xA7);
  image[0x18..0x20].copy_from_slice(&(content_size as u64).to_le_bytes());

  assert_eq!(round_trip(&image), image);
}

#[test]
fn refuses_resized_contents_without_a_new_tmd() {
  let image = builder().build();
  let mut artifacts = decode(&image);
  let content = artifacts
    .iter_mut()
    .find(|a| a.name == "content0.bin")
    .unwrap();
  content.data.truncate(0x1000);
  let err = N3dsCia
    .encode(Vec::new(), &artifacts, None)
    .map(|_| ())
    .expect_err("a resized content must fail while the TMD still records the old size")
    .to_string();
  assert!(err.contains("TMD"), "{err}");
}

#[test]
fn refuses_a_metadata_tmd_mismatch() {
  let image = builder().build();
  let mut artifacts = decode(&image);
  let mut metadata = CiaMetadata::from_json(&artifacts[0].data).unwrap();
  metadata.contents[0].size = 0x9999;
  artifacts[0].data = metadata.to_json().unwrap();
  let err = N3dsCia
    .encode(Vec::new(), &artifacts, None)
    .map(|_| ())
    .expect_err("a CIA.json that disagrees with tmd.bin must fail")
    .to_string();
  assert!(err.contains("does not match"), "{err}");
}

#[test]
fn explicit_hash_refresh_updates_tmd_tree_but_not_signature() {
  const REFRESH_HASHES: &str = r#"{"update_content_hashes":true}"#;
  let image = builder().build();
  assert_eq!(
    N3dsCia
      .encode(Vec::new(), &decode(&image), Some(REFRESH_HASHES))
      .unwrap(),
    image
  );
  let mut artifacts = decode(&image);
  let content = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "content0.bin")
    .unwrap();
  content.data[0x61] ^= 1;

  assert!(N3dsCia.encode(Vec::new(), &artifacts, None).is_err());
  let rebuilt = N3dsCia
    .encode(Vec::new(), &artifacts, Some(REFRESH_HASHES))
    .unwrap();
  let rebuilt_artifacts = decode(&rebuilt);
  let rebuilt_content = rebuilt_artifacts
    .iter()
    .find(|artifact| artifact.name == "content0.bin")
    .unwrap();
  let rebuilt_tmd = &rebuilt_artifacts
    .iter()
    .find(|artifact| artifact.name == TMD_NAME)
    .unwrap()
    .data;
  let record = parse_tmd_contents(rebuilt_tmd).unwrap().remove(0);
  assert_eq!(record.hash, hex::encode(sha256(&rebuilt_content.data)));

  let info_start = SIG_SIZE + TMD_CONTENT_INFO_OFFSET;
  let chunk_start = SIG_SIZE + TMD_CHUNK_RECORDS_OFFSET;
  let chunk_end = chunk_start + TMD_CHUNK_RECORD_SIZE;
  assert_eq!(
    &rebuilt_tmd[info_start + 4..info_start + 0x24],
    &sha256(&rebuilt_tmd[chunk_start..chunk_end])
  );
  let info_end = info_start + TMD_CONTENT_INFO_RECORD_COUNT * TMD_CONTENT_INFO_RECORD_SIZE;
  assert_eq!(
    &rebuilt_tmd
      [SIG_SIZE + TMD_CONTENT_INFO_HASH_OFFSET..SIG_SIZE + TMD_CONTENT_INFO_HASH_OFFSET + 0x20],
    &sha256(&rebuilt_tmd[info_start..info_end])
  );

  let original_tmd = decode(&image)
    .into_iter()
    .find(|artifact| artifact.name == TMD_NAME)
    .unwrap();
  assert_eq!(&rebuilt_tmd[..SIG_SIZE], &original_tmd.data[..SIG_SIZE]);
}
