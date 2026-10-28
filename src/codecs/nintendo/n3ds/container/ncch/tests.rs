use super::*;
use crate::codecs::nintendo::n3ds::container::cfa::N3dsCfa;
use crate::codecs::nintendo::n3ds::container::cxi::N3dsCxi;
use crate::codecs::nintendo::n3ds::container::ncch::metadata::{
  METADATA_NAME, NcchMetadata, NcchVariableSection, NcchVariableSectionKind,
};
use crate::codecs::{Codec, DecodedArtifact};

const UNIT: usize = 0x200;
const EXPLICIT: Option<&str> = Some(r#"{"preserve_padding":true}"#);
const PADDING_BYTE: u8 = 0xDD;

/// A deterministic byte pattern: byte `i` is `seed + i * (seed | 1)`, mod 256.
fn pattern(seed: u8, size: usize) -> Vec<u8> {
  (0..size)
    .map(|offset| seed.wrapping_add((offset as u8).wrapping_mul(seed | 1)))
    .collect()
}

fn write_u32(image: &mut [u8], offset: usize, value: u32) {
  image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_region(image: &mut [u8], base: usize, offset_units: u32, size_units: u32) {
  write_u32(image, base, offset_units);
  write_u32(image, base + 4, size_units);
}

/// A cleartext NCCH image with four regions and four uniform gaps:
///
/// ```text
/// 0x0200 .. 0x0400   gap (0xDD)
/// 0x0400 .. 0x0800   plain region   (2 units)
/// 0x0800 .. 0x0A00   gap (0xDD)
/// 0x0A00 .. 0x0C00   logo region    (1 unit)
/// 0x0C00 .. 0x1400   ExeFS          (4 units)
/// 0x1400 .. 0x1600   gap (0xDD)
/// 0x1600 .. 0x1A00   RomFS          (2 units)
/// 0x1A00 .. 0x1C00   gap (0xDD)
/// ```
fn image(no_crypto: bool) -> Vec<u8> {
  const IMAGE_LEN: usize = 0x1C00;

  let mut image = vec![0u8; IMAGE_LEN];
  image[0x100..0x104].copy_from_slice(NCCH_MAGIC);
  write_u32(&mut image, 0x104, (IMAGE_LEN / UNIT) as u32);
  write_region(&mut image, PLAIN_OFFSET, 2, 2);
  write_region(&mut image, LOGO_OFFSET, 5, 1);
  write_region(&mut image, EXEFS_OFFSET, 6, 4);
  write_region(&mut image, ROMFS_OFFSET, 11, 2);
  write_u32(&mut image, EXEFS_HASH_REGION_SIZE, 1);
  write_u32(&mut image, ROMFS_HASH_REGION_SIZE, 1);
  image[0x188] = 0; // media unit flag 0: 0x200-byte units
  image[0x18F] = if no_crypto { 0x04 } else { 0 };

  image[0x200..0x400].fill(PADDING_BYTE);
  image[0x400..0x800].copy_from_slice(&pattern(0x11, 2 * UNIT));
  image[0x800..0xA00].fill(PADDING_BYTE);
  image[0xA00..0xC00].copy_from_slice(&pattern(0x22, UNIT));
  image[0xC00..0x1400].copy_from_slice(&pattern(0x33, 4 * UNIT));
  image[0x1400..0x1600].fill(PADDING_BYTE);
  image[0x1600..0x1A00].copy_from_slice(&pattern(0x44, 2 * UNIT));
  image[0x1A00..0x1C00].fill(PADDING_BYTE);

  // The region hashes a cleartext image records, so a rebuild is an identity.
  let logo_hash = digest(&image[0xA00..0xC00]);
  let exefs_hash = digest(&image[0xC00..0xE00]);
  let romfs_hash = digest(&image[0x1600..0x1800]);
  image[0x130..0x150].copy_from_slice(&logo_hash);
  image[0x1C0..0x1E0].copy_from_slice(&exefs_hash);
  image[0x1E0..0x200].copy_from_slice(&romfs_hash);

  image
}

/// A cleartext NCCH image with no unclaimed intervals at all:
///
/// ```text
/// 0x0200 .. 0x0600   plain region   (2 units)
/// 0x0600 .. 0x0800   logo region    (1 unit)
/// 0x0800 .. 0x1000   ExeFS          (4 units)
/// 0x1000 .. 0x1400   RomFS          (2 units)
/// ```
fn packed_image(no_crypto: bool) -> Vec<u8> {
  const IMAGE_LEN: usize = 0x1400;

  let mut image = vec![0u8; IMAGE_LEN];
  image[0x100..0x104].copy_from_slice(NCCH_MAGIC);
  write_u32(&mut image, 0x104, (IMAGE_LEN / UNIT) as u32);
  write_region(&mut image, PLAIN_OFFSET, 1, 2);
  write_region(&mut image, LOGO_OFFSET, 3, 1);
  write_region(&mut image, EXEFS_OFFSET, 4, 4);
  write_region(&mut image, ROMFS_OFFSET, 8, 2);
  write_u32(&mut image, EXEFS_HASH_REGION_SIZE, 1);
  write_u32(&mut image, ROMFS_HASH_REGION_SIZE, 1);
  image[0x188] = 0; // media unit flag 0: 0x200-byte units
  image[0x18F] = if no_crypto { 0x04 } else { 0 };

  image[0x200..0x600].copy_from_slice(&pattern(0x11, 2 * UNIT));
  image[0x600..0x800].copy_from_slice(&pattern(0x22, UNIT));
  image[0x800..0x1000].copy_from_slice(&pattern(0x33, 4 * UNIT));
  image[0x1000..0x1400].copy_from_slice(&pattern(0x44, 2 * UNIT));

  let logo_hash = digest(&image[0x600..0x800]);
  let exefs_hash = digest(&image[0x800..0xA00]);
  let romfs_hash = digest(&image[0x1000..0x1200]);
  image[0x130..0x150].copy_from_slice(&logo_hash);
  image[0x1C0..0x1E0].copy_from_slice(&exefs_hash);
  image[0x1E0..0x200].copy_from_slice(&romfs_hash);

  image
}

fn codec_for(image: &[u8]) -> &'static dyn Codec {
  if u32_le(image, EXHEADER_SIZE_OFFSET).unwrap() > 0 {
    &N3dsCxi
  } else {
    &N3dsCfa
  }
}

fn decode(image: &[u8], args: Option<&str>) -> Vec<DecodedArtifact> {
  codec_for(image).decode(image, args).unwrap()
}

fn encode(artifacts: &[DecodedArtifact]) -> Vec<u8> {
  let metadata = metadata_of(artifacts);
  let codec: &dyn Codec = if metadata
    .fixed_sections
    .iter()
    .any(|section| section.kind == NcchFixedSectionKind::Exheader)
  {
    &N3dsCxi
  } else {
    &N3dsCfa
  };
  codec.encode(Vec::new(), artifacts, None).unwrap()
}

fn metadata_of(artifacts: &[DecodedArtifact]) -> NcchMetadata {
  NcchMetadata::from_json(
    artifacts
      .iter()
      .find(|artifact| artifact.name == METADATA_NAME)
      .expect("decode should emit NCCH.json")
      .data
      .as_slice(),
  )
  .expect("metadata should parse")
}

fn rebuilt_metadata(image: &[u8], args: Option<&str>) -> NcchMetadata {
  metadata_of(&decode(image, args))
}

fn artifact<'a>(artifacts: &'a [DecodedArtifact], name: &str) -> &'a [u8] {
  &artifacts
    .iter()
    .find(|artifact| artifact.name == name)
    .unwrap_or_else(|| panic!("decode should emit {name}"))
    .data
}

fn replace(artifacts: &mut [DecodedArtifact], name: &str, data: Vec<u8>) {
  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == name)
    .unwrap_or_else(|| panic!("decode should emit {name}"))
    .data = data;
}

fn update_metadata(artifacts: &mut [DecodedArtifact], change: impl FnOnce(&mut NcchMetadata)) {
  let artifact = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == METADATA_NAME)
    .expect("decode should emit NCCH.json");
  let mut metadata = NcchMetadata::from_json(&artifact.data).expect("metadata should parse");
  change(&mut metadata);
  artifact.data = metadata.to_json().expect("metadata should serialize");
}

fn section(metadata: &NcchMetadata, kind: NcchVariableSectionKind) -> &NcchVariableSection {
  metadata
    .variable_sections
    .iter()
    .find(|section| section.kind == kind)
    .unwrap_or_else(|| panic!("metadata should have a {kind:?} section"))
}

#[test]
fn implicit_padding_is_not_extracted_and_records_the_first_unclaimed_byte() {
  let input = image(true);
  let artifacts = decode(&input, None);

  let names: Vec<&str> = artifacts
    .iter()
    .map(|artifact| artifact.name.as_str())
    .collect();
  assert_eq!(
    names,
    [
      METADATA_NAME,
      "plain_region.bin",
      "logo_region.bin",
      "exefs.bin",
      "romfs.bin",
    ]
  );

  let metadata = metadata_of(&artifacts);
  assert_eq!(metadata.byte_padding, Some(PADDING_BYTE));
  assert_eq!(
    metadata
      .variable_sections
      .iter()
      .filter(|section| section.kind == NcchVariableSectionKind::Padding)
      .count(),
    4,
    "the recorded layout keeps the unclaimed intervals"
  );

  // The gaps are reconstructed from the recorded layout and filled with the
  // recorded byte, so an untouched decode is an identity.
  assert_eq!(encode(&artifacts), input);
}

#[test]
fn explicit_padding_extracts_every_unclaimed_interval() {
  let input = image(true);
  let artifacts = decode(&input, EXPLICIT);

  let names: Vec<&str> = artifacts
    .iter()
    .map(|artifact| artifact.name.as_str())
    .collect();
  assert_eq!(
    names,
    [
      METADATA_NAME,
      "padding0.bin",
      "plain_region.bin",
      "padding1.bin",
      "logo_region.bin",
      "exefs.bin",
      "padding2.bin",
      "romfs.bin",
      "padding3.bin",
    ]
  );

  let metadata = metadata_of(&artifacts);
  assert_eq!(metadata.byte_padding, None);
  for index in 0..4 {
    let name = format!("padding{index}.bin");
    assert_eq!(artifact(&artifacts, &name), vec![PADDING_BYTE; UNIT]);
  }

  // Explicit padding packs the artifacts sequentially, which reproduces the
  // contiguous layout they were extracted from.
  assert_eq!(encode(&artifacts), input);
}

#[test]
fn preserve_padding_controls_decoding_only() {
  let input = image(true);

  // An implicit decode stays implicit even when an encode argument asks for
  // explicit padding, and the other way round.
  let implicit = decode(&input, None);
  let explicit = decode(&input, EXPLICIT);

  assert_eq!(
    codec_for(&input)
      .encode(Vec::new(), &implicit, EXPLICIT)
      .expect("implicit artifacts should encode"),
    input
  );
  assert_eq!(
    codec_for(&input)
      .encode(Vec::new(), &explicit, None)
      .expect("explicit artifacts should encode"),
    input
  );
}

#[test]
fn gap_free_padding_mode_follows_byte_padding() {
  let input = packed_image(true);

  // Even with no padding sections to extract, an implicit decode records a
  // byte_padding, so the two modes stay distinguishable in the metadata.
  let mut implicit = decode(&input, None);
  assert_eq!(
    metadata_of(&implicit).byte_padding,
    Some(DEFAULT_PADDING_BYTE)
  );

  let mut explicit = decode(&input, EXPLICIT);
  let metadata = metadata_of(&explicit);
  assert_eq!(metadata.byte_padding, None);
  assert!(
    metadata
      .variable_sections
      .iter()
      .all(|section| section.kind != NcchVariableSectionKind::Padding)
  );

  // Shrinking a region must follow the mode in byte_padding: implicit keeps
  // the allocation and the later sections in place, explicit packs everything
  // sequentially from the current artifact sizes.
  replace(&mut implicit, "plain_region.bin", pattern(0x55, UNIT));
  let implicit_rebuild = encode(&implicit);
  let implicit_metadata = rebuilt_metadata(&implicit_rebuild, None);
  assert_eq!(implicit_rebuild.len(), input.len());
  assert_eq!(
    section(&implicit_metadata, NcchVariableSectionKind::LogoRegion).position,
    0x600
  );

  replace(&mut explicit, "plain_region.bin", pattern(0x55, UNIT));
  let explicit_rebuild = encode(&explicit);
  let explicit_metadata = rebuilt_metadata(&explicit_rebuild, EXPLICIT);
  assert_eq!(explicit_rebuild.len(), input.len() - UNIT);
  assert_eq!(
    section(&explicit_metadata, NcchVariableSectionKind::LogoRegion).position,
    0x400
  );
  assert_eq!(
    section(&explicit_metadata, NcchVariableSectionKind::Exefs).position,
    0x600
  );
  assert_eq!(
    section(&explicit_metadata, NcchVariableSectionKind::Romfs).position,
    0xE00
  );
}

#[test]
fn implicit_shrinking_preserves_the_allocation_and_fills_the_gap() {
  let input = image(true);
  let mut artifacts = decode(&input, None);

  replace(&mut artifacts, "plain_region.bin", pattern(0x55, UNIT));
  let rebuilt = encode(&artifacts);

  assert_eq!(rebuilt.len(), input.len(), "the allocation is preserved");

  let metadata = rebuilt_metadata(&rebuilt, None);
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::PlainRegion).position,
    0x400
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::PlainRegion).size_available,
    UNIT as u64
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::LogoRegion).position,
    0xA00,
    "shrinking must not move the next region"
  );
  assert!(
    rebuilt[0x600..0x800]
      .iter()
      .all(|byte| *byte == PADDING_BYTE),
    "the hole left by the shrink is filled with byte_padding"
  );
}

#[test]
fn implicit_growth_within_the_allocation_does_not_move_later_sections() {
  let input = image(true);
  let mut artifacts = decode(&input, None);

  // Shrink first, then grow back into the recorded allocation.
  replace(&mut artifacts, "plain_region.bin", pattern(0x55, UNIT));
  let _ = encode(&artifacts);
  replace(&mut artifacts, "plain_region.bin", pattern(0x11, 2 * UNIT));

  let rebuilt = encode(&artifacts);
  let metadata = rebuilt_metadata(&rebuilt, None);

  assert_eq!(
    section(&metadata, NcchVariableSectionKind::LogoRegion).position,
    0xA00,
    "growth within size_available must not move the next region"
  );
  assert_eq!(rebuilt, input);
}

#[test]
fn implicit_growth_beyond_the_allocation_moves_later_sections() {
  let input = image(true);
  let mut artifacts = decode(&input, None);

  replace(&mut artifacts, "plain_region.bin", pattern(0x55, 3 * UNIT));
  update_metadata(&mut artifacts, |metadata| metadata.content_size += 1);

  let rebuilt = encode(&artifacts);
  assert_eq!(rebuilt.len(), input.len() + UNIT);
  let metadata = rebuilt_metadata(&rebuilt, None);

  assert_eq!(
    section(&metadata, NcchVariableSectionKind::PlainRegion).size_available,
    3 * UNIT as u64
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::LogoRegion).position,
    0xC00
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::Exefs).position,
    0xE00
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::Romfs).position,
    0x1800
  );
}

#[test]
fn explicit_shrinking_moves_later_sections() {
  let input = image(true);
  let mut artifacts = decode(&input, EXPLICIT);

  replace(&mut artifacts, "plain_region.bin", pattern(0x55, UNIT));
  let rebuilt = encode(&artifacts);

  assert_eq!(rebuilt.len(), input.len() - UNIT);
  let metadata = rebuilt_metadata(&rebuilt, EXPLICIT);

  assert_eq!(
    section(&metadata, NcchVariableSectionKind::LogoRegion).position,
    0x800
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::Exefs).position,
    0xA00
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::Romfs).position,
    0x1400
  );
}

#[test]
fn explicit_shrinking_padding_compensates_for_growth() {
  let input = image(true);
  let mut artifacts = decode(&input, EXPLICIT);

  replace(&mut artifacts, "plain_region.bin", pattern(0x55, 3 * UNIT));
  // Remove the gap between the plain region and the logo: the growth and the
  // shrink cancel out, so every later section keeps its position.
  replace(&mut artifacts, "padding1.bin", Vec::new());

  let rebuilt = encode(&artifacts);
  assert_eq!(rebuilt.len(), input.len());
  let metadata = rebuilt_metadata(&rebuilt, EXPLICIT);

  assert_eq!(
    section(&metadata, NcchVariableSectionKind::PlainRegion).size_available,
    3 * UNIT as u64
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::LogoRegion).position,
    0xA00
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::Exefs).position,
    0xC00
  );
  assert_eq!(
    section(&metadata, NcchVariableSectionKind::Romfs).position,
    0x1600
  );
}

#[test]
fn explicit_misaligned_final_region_is_rejected() {
  let input = image(true);
  let mut artifacts = decode(&input, EXPLICIT);

  let mut romfs = artifact(&artifacts, "romfs.bin").to_vec();
  romfs.extend(pattern(0x77, 0x100));
  replace(&mut artifacts, "romfs.bin", romfs);
  update_metadata(&mut artifacts, |metadata| metadata.content_size += 1);

  let error = codec_for(&input)
    .encode(Vec::new(), &artifacts, None)
    .expect_err("a misaligned final region must be refused");
  assert!(error.to_string().contains("media units"), "{error}");
}

#[test]
fn encrypted_region_modification_is_rejected() {
  let input = image(false);
  let mut artifacts = decode(&input, None);

  // Leaving an encrypted region untouched is fine: its recorded hash still
  // covers the bytes this codec carries.
  codec_for(&input)
    .encode(Vec::new(), &artifacts, None)
    .expect("an untouched encrypted image should rebuild");

  replace(&mut artifacts, "exefs.bin", pattern(0x99, 4 * UNIT));

  let error = codec_for(&input)
    .encode(Vec::new(), &artifacts, None)
    .expect_err("an encrypted region must not be replaced");
  assert!(error.to_string().contains("encrypted"), "{error}");
}
