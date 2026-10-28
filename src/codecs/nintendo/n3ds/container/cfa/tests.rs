use super::*;
use crate::codecs::nintendo::n3ds::container::ncch::metadata::{
  METADATA_NAME, NcchMetadata, NcchVariableSectionKind,
};
use crate::codecs::nintendo::n3ds::container::ncch::testkit::build;
use crate::codecs::{Codec, CodecHandlingConfidence, DecodedArtifact};

const EXPLICIT: Option<&str> = Some(r#"{"preserve_padding":true}"#);

fn cfa() -> Vec<u8> {
  build(
    0,
    &[
      ("exefs.bin", 0x2, 4),
      ("romfs.bin", 0x6, 8),
    ],
  )
}

fn decode(image: &[u8]) -> Vec<DecodedArtifact> {
  N3dsCfa.decode(image, None).unwrap()
}

#[test]
fn detects_cfa_and_rejects_cxi() {
  assert_eq!(
    N3dsCfa.can_handle(&cfa()).unwrap(),
    CodecHandlingConfidence::Certain
  );
  let cxi = build(0x400, &[("exefs.bin", 0x10, 4)]);
  assert_eq!(
    N3dsCfa.can_handle(&cxi).unwrap(),
    CodecHandlingConfidence::No
  );
}

#[test]
fn round_trips_a_cfa() {
  let image = cfa();
  let artifacts = decode(&image);
  let names: Vec<&str> = artifacts.iter().map(|a| a.name.as_str()).collect();
  assert_eq!(
    names,
    [
      METADATA_NAME,
      "exefs.bin",
      "romfs.bin"
    ]
  );
  let rebuilt = N3dsCfa.encode(Vec::new(), &artifacts, None).unwrap();
  assert_eq!(rebuilt, image);
}

#[test]
fn preserves_nonuniform_padding_when_requested() {
  let mut image = build(
    0,
    &[
      ("exefs.bin", 0x4, 4),
      ("romfs.bin", 0xA, 8),
    ],
  );
  for (index, byte) in image[0x200..0x800].iter_mut().enumerate() {
    *byte = 0x31u8.wrapping_add(index as u8);
  }
  for (index, byte) in image[0x1000..0x1400].iter_mut().enumerate() {
    *byte = 0xA7u8.wrapping_sub(index as u8);
  }

  let artifacts = N3dsCfa.decode(&image, EXPLICIT).unwrap();
  let padding: Vec<_> = artifacts
    .iter()
    .filter(|artifact| artifact.name.starts_with("padding"))
    .map(|artifact| artifact.data.as_slice())
    .collect();
  assert_eq!(
    padding,
    [
      &image[0x200..0x800],
      &image[0x1000..0x1400]
    ]
  );
  assert_eq!(N3dsCfa.encode(Vec::new(), &artifacts, None).unwrap(), image);
}

#[test]
fn refreshes_hash_after_modifying_cfa_exefs() {
  let image = cfa();
  let mut artifacts = decode(&image);
  let exefs = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "exefs.bin")
    .unwrap();
  exefs.data[0x10] ^= 0xFF;
  let expected_exefs = exefs.data.clone();

  let rebuilt = N3dsCfa.encode(Vec::new(), &artifacts, None).unwrap();

  assert_eq!(
    rebuilt[..0x100],
    image[..0x100],
    "preserve the RSA signature"
  );
  assert_eq!(rebuilt[0x400..0xC00], expected_exefs);
  assert_eq!(
    &rebuilt[0x1C0..0x1E0],
    &crate::codecs::common::hash::sha256(&rebuilt[0x400..0x600]),
    "refresh the ExeFS superblock hash over the modified cleartext"
  );
}

#[test]
fn rebuilds_after_a_romfs_resize() {
  let image = cfa();
  let mut artifacts = decode(&image);
  let romfs = artifacts
    .iter_mut()
    .find(|a| a.name == "romfs.bin")
    .unwrap();
  romfs.data.truncate(0x600);

  let rebuilt = N3dsCfa.encode(Vec::new(), &artifacts, None).unwrap();
  let metadata = NcchMetadata::from_json(&decode(&rebuilt)[0].data).unwrap();
  let romfs = metadata
    .variable_sections
    .iter()
    .find(|section| section.kind == NcchVariableSectionKind::Romfs)
    .unwrap();
  assert_eq!(romfs.size_available, 0x600);
  assert_eq!(rebuilt.len(), image.len());
}

#[test]
fn rejects_a_cxi() {
  let cxi = build(0x400, &[("exefs.bin", 0x10, 4)]);
  let err = N3dsCfa
    .decode(&cxi, None)
    .map(|_| ())
    .expect_err("a CXI is not a CFA")
    .to_string();
  assert!(err.contains("not the requested"), "{err}");
}
