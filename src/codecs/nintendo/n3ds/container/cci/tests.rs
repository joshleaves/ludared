use super::*;
use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::DecodedArtifact;
use metadata::CARD_INFO_NAME;
use metadata::METADATA_NAME;
use metadata::NcsdMetadata;
use metadata::PartitionMetadata;
use metadata::TRAILER_NAME;
use metadata::partition_name;

const UNIT: usize = 0x200;

fn anonymous_bytes(index: usize, size: usize) -> Vec<u8> {
  (0..size)
    .map(|offset| (index as u8).wrapping_mul(31).wrapping_add(offset as u8))
    .collect()
}

fn partition_bytes(index: usize, size: usize) -> Vec<u8> {
  let mut bytes = anonymous_bytes(index, size);
  bytes[0x100..0x104].copy_from_slice(NCCH_MAGIC);
  bytes
}

fn signature() -> [u8; 0x100] {
  std::array::from_fn(|i| i as u8)
}

fn exheader_hash() -> [u8; EXHEADER_HASH_SIZE] {
  std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1))
}

fn reserved() -> [u8; RESERVED_SIZE] {
  std::array::from_fn(|i| 0xF0u8.wrapping_add(i as u8))
}

fn partition_ids() -> [u64; PARTITION_COUNT] {
  std::array::from_fn(|i| 0x0100_0000_0000_0000 + i as u64)
}

struct Builder {
  media_id: u64,
  image_size: Option<u32>,
  flags: [u8; 8],
  card_info: Vec<u8>,
  partitions: Vec<(usize, Vec<u8>)>,
  trailer: Vec<u8>,
  table: Option<[(u32, u32); PARTITION_COUNT]>,
}

fn builder() -> Builder {
  let mut flags = [0; 8];
  flags[FLAG_MEDIA_PLATFORM] = MEDIA_PLATFORM_CTR;
  flags[FLAG_MEDIA_TYPE] = MEDIA_TYPE_CARD1;
  Builder {
    media_id: 0x0004_0000_1234_5678,
    image_size: None,
    flags,
    card_info: vec![0xFF; FIRST_PARTITION_OFFSET as usize - HEADER_SIZE],
    partitions: Vec::new(),
    trailer: Vec::new(),
    table: None,
  }
}

impl Builder {
  fn partition(mut self, index: usize, size: usize) -> Self {
    self.partitions.push((index, partition_bytes(index, size)));
    self
  }

  fn anonymous(mut self, index: usize, size: usize) -> Self {
    self.partitions.push((index, anonymous_bytes(index, size)));
    self
  }

  fn media_id(mut self, id: u64) -> Self {
    self.media_id = id;
    self
  }

  fn media_type(mut self, kind: u8) -> Self {
    self.flags[FLAG_MEDIA_TYPE] = kind;
    self
  }

  fn media_platform(mut self, platform: u8) -> Self {
    self.flags[FLAG_MEDIA_PLATFORM] = platform;
    self
  }

  fn media_unit_flag(mut self, flag: u8) -> Self {
    self.flags[FLAG_MEDIA_UNIT] = flag;
    self
  }

  fn image_size(mut self, units: u32) -> Self {
    self.image_size = Some(units);
    self
  }

  fn trailer(mut self, bytes: Vec<u8>) -> Self {
    self.trailer = bytes;
    self
  }

  fn table(mut self, entries: [(u32, u32); PARTITION_COUNT]) -> Self {
    self.table = Some(entries);
    self
  }

  fn build(&self) -> Vec<u8> {
    let unit = media_unit_size(self.flags[FLAG_MEDIA_UNIT]).expect("valid fixture media unit");
    let mut computed = [(0, 0); PARTITION_COUNT];
    let mut running = FIRST_PARTITION_OFFSET;
    let mut body = Vec::new();

    for (index, data) in &self.partitions {
      computed[*index] = ((running / unit) as u32, (data.len() as u64 / unit) as u32);
      running += data.len() as u64;
      body.extend_from_slice(data);
    }

    let table = self.table.unwrap_or(computed);
    let image_size = self.image_size.unwrap_or_else(|| {
      let used = (running / unit) as u32;
      *KNOWN_IMAGE_SIZES
        .iter()
        .find(|size| **size >= used)
        .unwrap_or(&used)
    });

    let mut header = [0; HEADER_SIZE];
    header[..0x100].copy_from_slice(&signature());
    header[MAGIC_OFFSET..MAGIC_OFFSET + NCSD_MAGIC.len()].copy_from_slice(NCSD_MAGIC);
    header[0x104..0x108].copy_from_slice(&image_size.to_le_bytes());
    header[0x108..0x110].copy_from_slice(&self.media_id.to_le_bytes());
    header[0x110..0x118].fill(1);
    header[EXHEADER_HASH_OFFSET..EXHEADER_HASH_OFFSET + EXHEADER_HASH_SIZE]
      .copy_from_slice(&exheader_hash());
    header[0x188..0x190].copy_from_slice(&self.flags);

    for (index, (offset, size)) in table.iter().enumerate() {
      let base = PARTITION_TABLE_OFFSET + index * 8;
      header[base..base + 4].copy_from_slice(&offset.to_le_bytes());
      header[base + 4..base + 8].copy_from_slice(&size.to_le_bytes());
    }

    for (index, id) in partition_ids().iter().enumerate() {
      let base = PARTITION_ID_TABLE_OFFSET + index * 8;
      header[base..base + 8].copy_from_slice(&id.to_le_bytes());
    }

    header[RESERVED_OFFSET..RESERVED_OFFSET + RESERVED_SIZE].copy_from_slice(&reserved());

    let mut image = Vec::new();
    image.extend_from_slice(&header);
    image.extend_from_slice(&self.card_info);
    image.extend_from_slice(&body);
    image.extend_from_slice(&self.trailer);
    image
  }
}

fn names(artifacts: &[DecodedArtifact]) -> Vec<&str> {
  artifacts.iter().map(|a| a.name.as_str()).collect()
}

fn metadata_of(artifacts: &[DecodedArtifact]) -> NcsdMetadata {
  NcsdMetadata::from_json(&artifacts[0].data).unwrap()
}

fn edit_metadata(artifacts: &mut [DecodedArtifact], edit: impl FnOnce(&mut NcsdMetadata)) {
  let mut metadata = metadata_of(artifacts);
  edit(&mut metadata);
  artifacts[0].data = metadata.to_json().unwrap();
}

fn decode(image: &[u8]) -> Vec<DecodedArtifact> {
  N3dsCci.decode(image, None).unwrap()
}

fn round_trip(image: &[u8]) -> Vec<u8> {
  N3dsCci.encode(Vec::new(), &decode(image), None).unwrap()
}

fn decode_err(image: &[u8]) -> String {
  match N3dsCci.decode(image, None) {
    Err(err) => err.to_string(),
    Ok(_) => panic!("decode should fail"),
  }
}

fn encode_err(artifacts: &[DecodedArtifact]) -> String {
  N3dsCci
    .encode(Vec::new(), artifacts, None)
    .expect_err("encode should fail")
    .to_string()
}

fn find<'a>(artifacts: &'a mut [DecodedArtifact], name: &str) -> &'a mut DecodedArtifact {
  artifacts
    .iter_mut()
    .find(|a| a.name == name)
    .expect("fixture artifact")
}

#[test]
fn detects_valid_images() {
  for image in [
    builder().partition(0, 0x2000).build(),
    builder().partition(0, 0x2000).partition(1, 0x4000).build(),
    builder().partition(0, 0x4000).media_unit_flag(1).build(),
  ] {
    assert_eq!(
      N3dsCci.can_handle(&image).unwrap(),
      CodecHandlingConfidence::Certain
    );
  }
}

#[test]
fn reports_lower_confidence_for_unusual_images() {
  let unknown_capacity = builder().partition(0, 0x2000).image_size(0x12345).build();
  assert_eq!(
    N3dsCci.can_handle(&unknown_capacity).unwrap(),
    CodecHandlingConfidence::Likely,
  );

  let no_ncch = builder().anonymous(0, 0x2000).image_size(0x12345).build();
  assert_eq!(
    N3dsCci.can_handle(&no_ncch).unwrap(),
    CodecHandlingConfidence::No
  );
}

#[test]
fn rejects_non_ncsd_inputs() {
  for image in [
    Vec::new(),
    vec![0; FIRST_PARTITION_OFFSET as usize],
    vec![0xFF; FIRST_PARTITION_OFFSET as usize],
  ] {
    assert_eq!(
      N3dsCci.can_handle(&image).unwrap(),
      CodecHandlingConfidence::No
    );
  }
  assert!(
    N3dsCci
      .decode(&vec![0; FIRST_PARTITION_OFFSET as usize], None)
      .is_err()
  );

  let image = builder().partition(0, 0x2000).build();
  assert_eq!(
    N3dsCci.can_handle(&image[..HEADER_SIZE - 1]).unwrap(),
    CodecHandlingConfidence::No,
  );
}

#[test]
fn rejects_non_cartridge_images() {
  for image in [
    builder().partition(0, 0x2000).media_type(0).build(),
    builder().partition(0, 0x2000).media_id(0).build(),
    builder().partition(0, 0x2000).media_platform(2).build(),
  ] {
    assert_eq!(
      N3dsCci.can_handle(&image).unwrap(),
      CodecHandlingConfidence::No
    );
  }
}

#[test]
fn rejects_overflowing_partition_offset() {
  let mut table = [(0, 0); PARTITION_COUNT];
  table[0] = (u32::MAX, 0x10);
  let image = builder()
    .partition(0, 0x2000)
    .media_unit_flag(32)
    .table(table)
    .build();
  assert_eq!(
    N3dsCci.can_handle(&image).unwrap(),
    CodecHandlingConfidence::No
  );
  assert!(decode_err(&image).contains("overflows a 64-bit size"));
}

#[test]
fn decodes_header_and_partitions() {
  let image = builder().partition(0, 0x4000).partition(1, 0x2000).build();
  let artifacts = decode(&image);

  assert_eq!(
    names(&artifacts),
    [
      METADATA_NAME,
      CARD_INFO_NAME,
      partition_name(0).as_str(),
      partition_name(1).as_str()
    ],
  );

  let metadata = metadata_of(&artifacts);
  assert_eq!(metadata.media_id, 0x0004_0000_1234_5678);
  assert_eq!(metadata.signature, hex::encode(signature()));
  assert_eq!(metadata.exheader_hash, hex::encode(exheader_hash()));
  assert_eq!(metadata.header_reserved, hex::encode(reserved()));
  assert_eq!(metadata.partition_ids, partition_ids());
  assert_eq!(metadata.partition_fs_types, [1; 8]);
  assert_eq!(metadata.flags[FLAG_MEDIA_TYPE], MEDIA_TYPE_CARD1);
  assert_eq!(
    metadata.partitions,
    [
      PartitionMetadata {
        index: 0,
        offset: 0x20,
        size: 0x20
      },
      PartitionMetadata {
        index: 1,
        offset: 0x40,
        size: 0x10
      },
    ],
  );
  assert_eq!(
    artifacts[1].data,
    vec![0xFF; FIRST_PARTITION_OFFSET as usize - HEADER_SIZE],
  );
  assert_eq!(artifacts[2].data, partition_bytes(0, 0x4000));
  assert_eq!(artifacts[3].data, partition_bytes(1, 0x2000));
}

#[test]
fn round_trips_synthetic_images() {
  for image in [
    builder().partition(0, 0x2000).build(),
    builder().partition(0, 0x4000).partition(1, 0x2000).build(),
    builder().partition(0, 0x4000).media_unit_flag(1).build(),
  ] {
    assert_eq!(round_trip(&image), image);
  }
}

#[test]
fn preserves_trailing_bytes_and_trimmed_images() {
  let full = builder()
    .partition(0, 0x2000)
    .image_size(0x40)
    .trailer(vec![0xFF; 0x2000])
    .build();
  assert_eq!(full.len(), 0x40 * UNIT);
  let artifacts = decode(&full);
  assert_eq!(
    names(&artifacts),
    [
      METADATA_NAME,
      CARD_INFO_NAME,
      partition_name(0).as_str(),
      TRAILER_NAME
    ],
  );
  assert_eq!(artifacts.last().unwrap().data, vec![0xFF; 0x2000]);
  assert_eq!(round_trip(&full), full);

  let trimmed = builder().partition(0, 0x2000).image_size(0x40).build();
  assert_eq!(trimmed.len(), 0x6000);
  assert_eq!(
    names(&decode(&trimmed)),
    [
      METADATA_NAME,
      CARD_INFO_NAME,
      partition_name(0).as_str()
    ],
  );
  assert_eq!(round_trip(&trimmed), trimmed);
}

#[test]
fn rejects_bytes_beyond_declared_image_size() {
  let image = builder()
    .partition(0, 0x2000)
    .image_size(0x30)
    .trailer(vec![0xFF; 0x4000])
    .build();
  assert_eq!(image.len(), 0xA000);
  assert_eq!(
    N3dsCci.can_handle(&image).unwrap(),
    CodecHandlingConfidence::No
  );
  assert!(decode_err(&image).contains("past its declared image size"));
}

#[test]
fn recalculates_layout_when_partitions_change_size() {
  let image = builder().partition(0, 0x4000).partition(1, 0x4000).build();
  let mut artifacts = decode(&image);
  find(&mut artifacts, &partition_name(0))
    .data
    .truncate(0x3E00);
  let rebuilt = N3dsCci.encode(Vec::new(), &artifacts, None).unwrap();
  assert_eq!(rebuilt.len(), image.len() - UNIT);
  let metadata = metadata_of(&decode(&rebuilt));
  assert_eq!(metadata.partitions[0].size, 0x1F);
  assert_eq!(metadata.partitions[1].offset, 0x3F);
  let offset = FIRST_PARTITION_OFFSET as usize + 0x3E00;
  assert_eq!(
    &rebuilt[offset..offset + 0x4000],
    partition_bytes(1, 0x4000)
  );

  let image = builder().partition(0, 0x2000).partition(1, 0x2000).build();
  let mut artifacts = decode(&image);
  find(&mut artifacts, &partition_name(0))
    .data
    .extend_from_slice(&[0xAA; 0x400]);
  let rebuilt = N3dsCci.encode(Vec::new(), &artifacts, None).unwrap();
  assert_eq!(rebuilt.len(), image.len() + 0x400);
  let metadata = metadata_of(&decode(&rebuilt));
  assert_eq!(metadata.partitions[0].size, 0x12);
  assert_eq!(metadata.partitions[1].offset, 0x32);
}

#[test]
fn accepts_artifacts_in_any_order() {
  let image = builder().partition(0, 0x2000).partition(1, 0x2000).build();
  let mut artifacts = decode(&image);
  artifacts.reverse();
  assert_eq!(N3dsCci.encode(Vec::new(), &artifacts, None).unwrap(), image);
}

#[test]
fn enforces_declared_capacity_during_rebuild() {
  let image = builder().partition(0, 0x2000).image_size(0x30).build();
  let mut artifacts = decode(&image);
  find(&mut artifacts, &partition_name(0))
    .data
    .extend_from_slice(&[0; UNIT]);
  assert!(encode_err(&artifacts).contains("past the declared image size"));

  let image = builder()
    .partition(0, 0x2000)
    .image_size(0x40)
    .trailer(vec![0xFF; 0x2000])
    .build();
  let mut artifacts = decode(&image);
  find(&mut artifacts, &partition_name(0))
    .data
    .extend_from_slice(&[0; UNIT]);
  let message = encode_err(&artifacts);
  assert!(
    message.contains("past the declared image size"),
    "{message}"
  );
  assert!(message.contains("trailing region"), "{message}");

  edit_metadata(&mut artifacts, |m| m.image_size = 0x41);
  let rebuilt = N3dsCci.encode(Vec::new(), &artifacts, None).unwrap();
  assert_eq!(rebuilt.len(), 0x8200);
  let mut rebuilt_artifacts = decode(&rebuilt);
  assert_eq!(metadata_of(&rebuilt_artifacts).image_size, 0x41);
  assert_eq!(
    find(&mut rebuilt_artifacts, TRAILER_NAME).data,
    vec![0xFF; 0x2000]
  );
}
