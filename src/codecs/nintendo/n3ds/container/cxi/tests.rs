use super::*;
use crate::codecs::nintendo::n3ds::container::ncch::metadata::{
  METADATA_NAME, NcchMetadata, NcchVariableSectionKind,
};
use crate::codecs::nintendo::n3ds::container::ncch::testkit::{UNIT, build};
use crate::codecs::{Codec, CodecHandlingConfidence, DecodedArtifact};

fn cxi() -> Vec<u8> {
  build(
    0x400,
    &[
      ("plain.bin", 0x10, 1),
      ("logo.bin", 0x12, 0x10),
      ("exefs.bin", 0x22, 4),
      ("romfs.bin", 0x26, 8),
    ],
  )
}

fn decode(image: &[u8]) -> Vec<DecodedArtifact> {
  N3dsCxi.decode(image, None).unwrap()
}

fn decode_preserving_padding(image: &[u8]) -> Vec<DecodedArtifact> {
  N3dsCxi
    .decode(image, Some(r#"{"preserve_padding":true}"#))
    .unwrap()
}

fn assert_same_image(actual: &[u8], expected: &[u8]) {
  let differences: Vec<_> = actual
    .iter()
    .zip(expected)
    .enumerate()
    .filter_map(|(offset, (actual, expected))| (actual != expected).then_some(offset))
    .take(16)
    .collect();
  assert_eq!(actual.len(), expected.len(), "image lengths differ");
  assert!(
    differences.is_empty(),
    "first differing offsets: {differences:#x?}"
  );
}

#[test]
fn detects_cxi_and_rejects_cfa_and_foreign_data() {
  assert_eq!(
    N3dsCxi.can_handle(&cxi()).unwrap(),
    CodecHandlingConfidence::Certain
  );
  let cfa = build(0, &[("romfs.bin", 0x4, 8)]);
  assert_eq!(
    N3dsCxi.can_handle(&cfa).unwrap(),
    CodecHandlingConfidence::No
  );
  assert_eq!(
    N3dsCxi.can_handle(&vec![0; 0x4000]).unwrap(),
    CodecHandlingConfidence::No
  );
}

#[test]
fn round_trips_a_cxi() {
  let image = cxi();
  let artifacts = decode(&image);
  let names: Vec<&str> = artifacts.iter().map(|a| a.name.as_str()).collect();
  assert_eq!(
    names,
    [
      METADATA_NAME,
      "exheader.bin",
      "access_descriptor.bin",
      "plain_region.bin",
      "logo_region.bin",
      "exefs.bin",
      "romfs.bin"
    ]
  );
  let metadata = NcchMetadata::from_json(&artifacts[0].data).unwrap();
  assert!(metadata.fixed_sections.iter().any(|section| {
    section.kind
      == crate::codecs::nintendo::n3ds::container::ncch::metadata::NcchFixedSectionKind::Exheader
  }));
  assert_eq!(metadata.partition_id, 0x0004_0000_1234_5678);
  assert_same_image(
    &N3dsCxi.encode(Vec::new(), &artifacts, None).unwrap(),
    &image,
  );
}

#[test]
fn preserves_extended_header_and_region_gaps_in_physical_order() {
  let image = build(
    0x400,
    &[
      ("plain.bin", 0x15, 1),
      ("logo.bin", 0x05, 0x10),
      ("exefs.bin", 0x16, 4),
      ("romfs.bin", 0x1C, 8),
    ],
  );
  let mut image = image;
  for (index, byte) in image[0x600..0xA00].iter_mut().enumerate() {
    *byte = (index as u8).wrapping_mul(13).wrapping_add(1);
  }
  for (index, byte) in image[0x3400..0x3800].iter_mut().enumerate() {
    *byte = (index as u8).wrapping_mul(7).wrapping_add(3);
  }
  // Updating the exheader invalidates its documented hash; the synthetic
  // fixture has no signing key, so use matching metadata for this layout test.
  let exheader_hash = crate::codecs::common::hash::sha256(&image[0x200..0x600]);
  image[0x160..0x180].copy_from_slice(&exheader_hash);

  let artifacts = decode_preserving_padding(&image);
  let metadata = NcchMetadata::from_json(&artifacts[0].data).unwrap();
  assert!(metadata.variable_sections.iter().any(|section| {
    section.kind == NcchVariableSectionKind::Padding && section.position == 0x3400
  }));
  assert_eq!(
    artifacts
      .iter()
      .find(|artifact| artifact.name == "exheader.bin")
      .unwrap()
      .data,
    image[0x200..0x600]
  );
  assert_eq!(
    artifacts
      .iter()
      .map(|artifact| artifact.name.as_str())
      .collect::<Vec<_>>(),
    [
      METADATA_NAME,
      "exheader.bin",
      "access_descriptor.bin",
      "logo_region.bin",
      "plain_region.bin",
      "exefs.bin",
      "padding0.bin",
      "romfs.bin"
    ]
  );
  assert_same_image(
    &N3dsCxi.encode(Vec::new(), &artifacts, None).unwrap(),
    &image,
  );
}

#[test]
fn refuses_malformed_fixed_width_metadata_without_panicking() {
  let mut artifacts = decode(&cxi());
  let mut metadata = NcchMetadata::from_json(&artifacts[0].data).unwrap();
  metadata.signature.truncate(2);
  artifacts[0].data = metadata.to_json().unwrap();

  let Err(error) = N3dsCxi.encode(Vec::new(), &artifacts, None) else {
    panic!("a short NCCH signature must be rejected");
  };
  let error = error.to_string();
  assert!(error.to_lowercase().contains("signature"), "{error}");
}

#[test]
fn recomputes_layout_and_hashes_after_a_region_grows() {
  let image = cxi();
  let mut artifacts = decode(&image);
  let exefs = artifacts
    .iter_mut()
    .find(|a| a.name == "exefs.bin")
    .unwrap();
  exefs.data.extend_from_slice(&[0xAA; UNIT]);
  let mut metadata = NcchMetadata::from_json(&artifacts[0].data).unwrap();
  metadata.content_size += 1;
  artifacts[0].data = metadata.to_json().unwrap();

  let rebuilt = N3dsCxi.encode(Vec::new(), &artifacts, None).unwrap();
  assert_eq!(rebuilt.len(), image.len() + UNIT);
  // The RomFS moved; the metadata of the rebuilt container shows it.
  let artifacts = decode(&rebuilt);
  let metadata = NcchMetadata::from_json(&artifacts[0].data).unwrap();
  let romfs = metadata
    .variable_sections
    .iter()
    .find(|section| section.kind == NcchVariableSectionKind::Romfs)
    .unwrap();
  assert_eq!(romfs.position / UNIT as u64, 0x26 + 1);
  assert_eq!(
    N3dsCxi.encode(Vec::new(), &artifacts, None).unwrap(),
    rebuilt
  );
}

#[test]
fn rejects_a_cfa() {
  let cfa = build(0, &[("romfs.bin", 0x4, 8)]);
  let err = N3dsCxi
    .decode(&cfa, None)
    .map(|_| ())
    .expect_err("a CFA is not a CXI")
    .to_string();
  assert!(err.contains("not the requested"), "{err}");
}

#[test]
fn rejects_truncated_containers() {
  let image = cxi();
  let truncated = image[..image.len() - UNIT].to_vec();
  assert!(N3dsCxi.decode(&truncated, None).is_err());
}
