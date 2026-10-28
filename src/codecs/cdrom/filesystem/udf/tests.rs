use super::volume::TAG_ANCHOR;
use super::volume::TAG_FILE_ENTRY;
use super::volume::TAG_FILE_IDENTIFIER;
use super::volume::TAG_FILE_SET;
use super::volume::TAG_LOGICAL_VOLUME;
use super::volume::TAG_PARTITION;
use super::volume::TAG_PRIMARY_VOLUME;
use super::volume::align4;
use super::volume::udf_crc16;
use super::*;

#[test]
fn physical_partition_image_round_trips_and_keeps_contents_separate() {
  let image = fixture();
  assert_eq!(
    Udf.can_handle(&image).unwrap(),
    CodecHandlingConfidence::Certain
  );

  let artifacts = Udf.decode(&image, None).unwrap();
  assert_eq!(artifacts[0].name, METADATA_NAME);
  assert_eq!(artifacts[1].name, "HELLO.TXT");
  assert_eq!(artifacts[1].data, b"hello");
  assert!(!artifacts[0].data.windows(5).any(|bytes| bytes == b"hello"));
  assert_eq!(Udf.encode(Vec::new(), &artifacts, None).unwrap(), image);

  let mut modified = artifacts;
  modified[1].data = b"hello, world!".to_vec();
  let rebuilt = Udf.encode(Vec::new(), &modified, None).unwrap();
  assert_eq!(
    &rebuilt[104 * BLOCK_SIZE..104 * BLOCK_SIZE + 13],
    b"hello, world!"
  );
  let rebuilt_artifacts = Udf.decode(&rebuilt, None).unwrap();
  assert_eq!(rebuilt_artifacts[1].data, b"hello, world!");
}

#[test]
fn restores_opaque_allocation_tail_before_writing_changed_file_bytes() {
  let mut image = fixture();
  image[104 * BLOCK_SIZE + 5..105 * BLOCK_SIZE].fill(0xD3);
  let artifacts = Udf.decode(&image, None).unwrap();
  assert_eq!(Udf.encode(Vec::new(), &artifacts, None).unwrap(), image);

  let mut grown = artifacts
    .iter()
    .map(|artifact| DecodedArtifact {
      name: artifact.name.clone(),
      data: artifact.data.clone(),
    })
    .collect::<Vec<_>>();
  grown[1].data = b"hello, world!".to_vec();
  let rebuilt = Udf.encode(Vec::new(), &grown, None).unwrap();
  assert_eq!(
    &rebuilt[104 * BLOCK_SIZE..104 * BLOCK_SIZE + 13],
    b"hello, world!"
  );
  assert!(
    rebuilt[104 * BLOCK_SIZE + 13..105 * BLOCK_SIZE]
      .iter()
      .all(|byte| *byte == 0xD3)
  );

  grown[1].data = b"hi".to_vec();
  let shrunk = Udf.encode(Vec::new(), &grown, None).unwrap();
  assert_eq!(&shrunk[104 * BLOCK_SIZE..104 * BLOCK_SIZE + 2], b"hi");
  assert_eq!(
    &shrunk[104 * BLOCK_SIZE + 5..105 * BLOCK_SIZE],
    &image[104 * BLOCK_SIZE + 5..105 * BLOCK_SIZE]
  );
}

#[test]
fn refuses_growth_past_existing_allocation_capacity() {
  let mut artifacts = Udf.decode(&fixture(), None).unwrap();
  artifacts[1].data = vec![0xA5; BLOCK_SIZE + 1];
  let error = Udf.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(
    error.to_string().contains("existing allocation holds 2048"),
    "{error}"
  );
}

#[test]
fn deduplicates_directory_entries_that_share_one_file_entry() {
  let mut image = fixture();
  add_file_identifier(&mut image, 88, "ALIAS.TXT", 3);
  let artifacts = Udf.decode(&image, None).unwrap();
  assert_eq!(
    artifacts
      .iter()
      .map(|artifact| artifact.name.as_str())
      .collect::<Vec<_>>(),
    [
      METADATA_NAME,
      "HELLO.TXT"
    ]
  );

  let metadata: Metadata = serde_json::from_slice(&artifacts[0].data).unwrap();
  assert_eq!(
    metadata.files[0].aliases,
    [
      "HELLO.TXT",
      "ALIAS.TXT"
    ]
  );
  assert_eq!(
    metadata.files[0].file_entry_offsets,
    [103 * BLOCK_SIZE as u64]
  );
}

#[test]
fn rejects_distinct_file_entries_that_share_data_extents() {
  let mut image = fixture();
  let second_entry = block_mut(&mut image, 105);
  second_entry[27] = 5;
  put_u64(second_entry, 56, 5);
  put_u32(second_entry, 172, 8);
  put_u32(second_entry, 176, BLOCK_SIZE as u32);
  put_u32(second_entry, 180, 4);
  finish_tag(second_entry, TAG_FILE_ENTRY, 5, 168);
  add_file_identifier(&mut image, 88, "COPY.TXT", 5);

  let Err(error) = Udf.decode(&image, None) else {
    panic!("overlapping allocations should be rejected");
  };
  assert!(
    error.to_string().contains("overlapping allocations"),
    "{error}"
  );
  assert!(
    error.to_string().contains("do not share a File Entry"),
    "{error}"
  );
}

#[test]
fn rejects_file_allocations_overlapping_udf_metadata() {
  let mut image = fixture();
  let file_entry = block_mut(&mut image, 103);
  put_u32(file_entry, 180, 1);
  finish_tag(file_entry, TAG_FILE_ENTRY, 3, 168);

  let Err(error) = Udf.decode(&image, None) else {
    panic!("allocations overlapping UDF metadata should be rejected");
  };
  assert!(
    error.to_string().contains("overlaps UDF File Entry"),
    "{error}"
  );
}

#[test]
fn refuses_tampered_metadata_that_redirects_file_writes_over_structures() {
  let mut artifacts = Udf.decode(&fixture(), None).unwrap();
  let mut metadata: Metadata = serde_json::from_slice(&artifacts[0].data).unwrap();
  metadata.files[0].extents[0].offset = 101 * BLOCK_SIZE as u64;
  artifacts[0].data = serde_json::to_vec(&metadata).unwrap();

  let Err(error) = Udf.encode(Vec::new(), &artifacts, None) else {
    panic!("metadata that redirects writes over UDF structures must be rejected");
  };
  assert!(error.to_string().contains("UDF"), "{error}");
}

#[test]
fn rejects_inline_data_instead_of_returning_a_partial_file() {
  let mut image = fixture();
  let file_entry = block_mut(&mut image, 103);
  put_u16(file_entry, 34, 3);
  finish_tag(file_entry, TAG_FILE_ENTRY, 3, 168);

  let Err(error) = Udf.decode(&image, None) else {
    panic!("inline file data should be rejected");
  };
  assert!(
    error
      .to_string()
      .contains("inline file data is unsupported"),
    "{error}"
  );
}

#[test]
fn bridge_growth_is_independently_readable_through_udf_and_iso9660() {
  let source = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/testing/fixtures/cdrom/udf_golden.iso"
  ));
  let mut artifacts = Udf.decode(source, None).unwrap();
  let shared = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "DATA.BIN")
    .unwrap();
  shared.data = vec![0xA5; 1200];
  let rebuilt = Udf.encode(Vec::new(), &artifacts, None).unwrap();

  let udf_file = read_udf(&rebuilt)
    .unwrap()
    .files
    .into_iter()
    .find(|file| file.path == "DATA.BIN")
    .unwrap();
  let iso_file = iso_volume::read(
    &rebuilt,
    crate::codecs::cdrom::filesystem::iso9660::OutOfBoundsEntries::Preserve,
  )
  .unwrap()
  .files
  .into_iter()
  .find(|file| file.name == "DATA.BIN")
  .unwrap();
  assert_eq!(udf_file.data, vec![0xA5; 1200]);
  assert_eq!(iso_file.data, vec![0xA5; 1200]);
}

#[test]
fn bridge_layout_read_skips_iso_file_content_copies() {
  let source = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/testing/fixtures/cdrom/udf_golden.iso"
  ));
  let full = iso_volume::read(
    source,
    crate::codecs::cdrom::filesystem::iso9660::OutOfBoundsEntries::Preserve,
  )
  .unwrap();
  let layout = iso_volume::read_layout(
    source,
    crate::codecs::cdrom::filesystem::iso9660::OutOfBoundsEntries::Preserve,
  )
  .unwrap();

  assert_eq!(layout.layout, full.layout);
  assert_eq!(layout.files.len(), full.files.len());
  for (layout_file, full_file) in layout.files.iter().zip(&full.files) {
    assert_eq!(layout_file.name, full_file.name);
    assert_eq!(layout_file.extent, full_file.extent);
    assert_eq!(layout_file.length, full_file.data.len());
    assert!(layout_file.data.is_empty());
  }
}

#[test]
fn refuses_bridge_growth_past_existing_udf_and_iso_allocations() {
  let source = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/testing/fixtures/cdrom/udf_golden.iso"
  ));
  let mut artifacts = Udf.decode(source, None).unwrap();
  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "DATA.BIN")
    .unwrap()
    .data = vec![0xA5; BLOCK_SIZE + 1];

  let Err(error) = Udf.encode(Vec::new(), &artifacts, None) else {
    panic!("bridge growth beyond the shared allocation must be rejected");
  };
  assert!(
    error.to_string().contains("existing allocation holds 2048"),
    "{error}"
  );
}

fn fixture() -> Vec<u8> {
  let mut image = vec![0; 300 * BLOCK_SIZE];
  image[17 * BLOCK_SIZE + 1..17 * BLOCK_SIZE + 6].copy_from_slice(b"NSR02");

  let anchor = block_mut(&mut image, 256);
  put_u32(anchor, 16, 16 * BLOCK_SIZE as u32);
  put_u32(anchor, 20, 32);
  finish_tag(anchor, TAG_ANCHOR, 256, 496);

  let primary = block_mut(&mut image, 32);
  finish_tag(primary, TAG_PRIMARY_VOLUME, 32, 496);

  let partition = block_mut(&mut image, 33);
  put_u16(partition, 22, 2);
  put_u32(partition, 188, 100);
  put_u32(partition, 192, 150);
  finish_tag(partition, TAG_PARTITION, 33, 496);

  let logical = block_mut(&mut image, 34);
  put_u32(logical, 212, BLOCK_SIZE as u32);
  logical[217..240].copy_from_slice(b"*OSTA UDF Compliant\0\0\0\0");
  logical[240..248].copy_from_slice(&[
    0x02,
    0x01,
    0,
    0,
    0,
    0,
    0,
    0,
  ]);
  put_u32(logical, 248, BLOCK_SIZE as u32);
  put_u32(logical, 252, 0);
  put_u16(logical, 256, 0);
  put_u32(logical, 264, 6);
  put_u32(logical, 268, 1);
  logical[440..446].copy_from_slice(&[1, 6, 1, 0, 2, 0]);
  finish_tag(logical, TAG_LOGICAL_VOLUME, 34, 430);

  let terminator = block_mut(&mut image, 35);
  finish_tag(terminator, 8, 35, 0);

  let fsd = block_mut(&mut image, 100);
  put_long_ad(fsd, 400, BLOCK_SIZE as u32, 1, 0);
  finish_tag(fsd, TAG_FILE_SET, 0, 496);

  let root_entry = block_mut(&mut image, 101);
  root_entry[27] = 4;
  put_u64(root_entry, 56, BLOCK_SIZE as u64);
  put_u32(root_entry, 172, 8);
  put_u32(root_entry, 176, BLOCK_SIZE as u32);
  put_u32(root_entry, 180, 2);
  finish_tag(root_entry, TAG_FILE_ENTRY, 1, 168);

  let file_entry = block_mut(&mut image, 103);
  file_entry[27] = 5;
  put_u64(file_entry, 56, 5);
  put_u32(file_entry, 172, 8);
  put_u32(file_entry, 176, BLOCK_SIZE as u32);
  put_u32(file_entry, 180, 4);
  finish_tag(file_entry, TAG_FILE_ENTRY, 3, 168);

  let directory = block_mut(&mut image, 102);
  let parent = &mut directory[..40];
  put_u16(parent, 16, 1);
  parent[18] = 0x0a;
  put_long_ad(parent, 20, BLOCK_SIZE as u32, 1, 0);
  finish_tag(parent, TAG_FILE_IDENTIFIER, 2, 24);

  let child = &mut directory[40..88];
  put_u16(child, 16, 1);
  child[19] = 10;
  put_long_ad(child, 20, BLOCK_SIZE as u32, 3, 0);
  child[38..48].copy_from_slice(b"\x08HELLO.TXT");
  finish_tag(child, TAG_FILE_IDENTIFIER, 2, 32);

  image[104 * BLOCK_SIZE..104 * BLOCK_SIZE + 5].copy_from_slice(b"hello");
  image
}

fn add_file_identifier(image: &mut [u8], offset: usize, name: &str, icb_block: u32) {
  let encoded_name = [
    vec![8],
    name.as_bytes().to_vec(),
  ]
  .concat();
  let size = align4(38 + encoded_name.len());
  let record = &mut image[102 * BLOCK_SIZE + offset..102 * BLOCK_SIZE + offset + size];
  put_u16(record, 16, 1);
  record[19] = encoded_name.len() as u8;
  put_long_ad(record, 20, BLOCK_SIZE as u32, icb_block, 0);
  record[38..38 + encoded_name.len()].copy_from_slice(&encoded_name);
  finish_tag(record, TAG_FILE_IDENTIFIER, 2, (size - 16) as u16);
}

fn block_mut(image: &mut [u8], index: usize) -> &mut [u8] {
  &mut image[index * BLOCK_SIZE..(index + 1) * BLOCK_SIZE]
}

fn put_u16(data: &mut [u8], offset: usize, value: u16) {
  data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(data: &mut [u8], offset: usize, value: u32) {
  data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(data: &mut [u8], offset: usize, value: u64) {
  data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn put_long_ad(data: &mut [u8], offset: usize, length: u32, block: u32, map: u16) {
  put_u32(data, offset, length);
  put_u32(data, offset + 4, block);
  put_u16(data, offset + 8, map);
}

fn finish_tag(data: &mut [u8], id: u16, location: u32, crc_len: u16) {
  put_u16(data, 0, id);
  put_u16(data, 2, 2);
  data[4] = 0;
  put_u16(data, 6, 1);
  put_u16(data, 8, 0);
  put_u16(data, 10, crc_len);
  put_u32(data, 12, location);
  if crc_len > 0 {
    let length = usize::from(crc_len);
    let crc = udf_crc16(&data[16..16 + length]);
    put_u16(data, 8, crc);
  }
  data[4] = data[..4]
    .iter()
    .chain(data[5..16].iter())
    .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
}
