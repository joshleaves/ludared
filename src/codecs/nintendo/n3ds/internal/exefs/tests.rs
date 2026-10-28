use super::*;
use crate::codecs::common::hash::sha256;
use crate::codecs::{Codec, CodecHandlingConfidence, DecodedArtifact};

fn file_bytes(seed: u8, size: usize) -> Vec<u8> {
  (0..size).map(|i| seed.wrapping_add(i as u8)).collect()
}

fn build(files: &[(&str, usize)]) -> Vec<u8> {
  let mut header = [0u8; HEADER_SIZE];
  let mut body = Vec::new();
  for (index, (name, size)) in files.iter().enumerate() {
    let offset = body.len() as u32;
    let bytes = file_bytes(index as u8 + 1, *size);
    header[index * FILE_HEADER_SIZE..index * FILE_HEADER_SIZE + 8]
      .copy_from_slice(&canonical_name(name));
    header[index * FILE_HEADER_SIZE + 0x8..index * FILE_HEADER_SIZE + 0xC]
      .copy_from_slice(&offset.to_le_bytes());
    header[index * FILE_HEADER_SIZE + 0xC..index * FILE_HEADER_SIZE + 0x10]
      .copy_from_slice(&(*size as u32).to_le_bytes());
    header[hash_slot(index)..hash_slot(index) + HASH_SIZE].copy_from_slice(&sha256(&bytes));
    body.extend_from_slice(&bytes);
    body.resize(body.len().next_multiple_of(FILE_ALIGN as usize), 0);
  }
  let mut image = header.to_vec();
  image.extend_from_slice(&body);
  image
}

fn decode(image: &[u8]) -> Vec<DecodedArtifact> {
  N3dsExeFs.decode(image, None).unwrap()
}

fn round_trip(image: &[u8]) -> Vec<u8> {
  N3dsExeFs.encode(Vec::new(), &decode(image), None).unwrap()
}

#[test]
fn detects_valid_images() {
  let image = build(&[
    ("icon", 0x100),
    (".code", 0x400),
  ]);
  assert_eq!(
    N3dsExeFs.can_handle(&image).unwrap(),
    CodecHandlingConfidence::Certain
  );
}

#[test]
fn rejects_foreign_data() {
  for image in [
    Vec::new(),
    vec![0; HEADER_SIZE],
    vec![0xFF; 0x400],
  ] {
    assert_eq!(
      N3dsExeFs.can_handle(&image).unwrap(),
      CodecHandlingConfidence::No
    );
  }
}

#[test]
fn lowers_confidence_when_hashes_do_not_match() {
  let mut image = build(&[("icon", 0x100)]);
  image[HEADER_SIZE] ^= 0xFF;
  assert_eq!(
    N3dsExeFs.can_handle(&image).unwrap(),
    CodecHandlingConfidence::Likely
  );
}

#[test]
fn round_trips() {
  for image in [
    build(&[("icon", 0x100)]),
    build(&[
      ("icon", 0x100),
      ("banner", 0x240),
      (".code", 0x1234),
    ]),
  ] {
    assert_eq!(round_trip(&image), image);
  }
}

#[test]
fn decodes_names_and_sizes() {
  let image = build(&[
    ("icon", 0x100),
    (".code", 0x400),
  ]);
  let artifacts = decode(&image);
  let names: Vec<&str> = artifacts.iter().map(|a| a.name.as_str()).collect();
  assert_eq!(
    names,
    [
      METADATA_NAME,
      "icon",
      ".code"
    ]
  );
  assert_eq!(artifacts[1].data, file_bytes(1, 0x100));
  assert_eq!(artifacts[2].data, file_bytes(2, 0x400));
}

#[test]
fn rejects_names_that_do_not_fit_the_exefs_name_field() {
  let mut artifacts = decode(&build(&[("icon", 0x100)]));
  let mut metadata = ExefsMetadata::from_json(&artifacts[0].data).unwrap();
  metadata.files[0].name = "123456789".to_owned();
  metadata.files[0].name_raw = None;
  artifacts[0].data = metadata.to_json().unwrap();
  artifacts[1].name = "123456789".to_owned();

  let Err(error) = N3dsExeFs.encode(Vec::new(), &artifacts, None) else {
    panic!("an ExeFS name longer than eight bytes must be rejected");
  };
  let error = error.to_string();
  assert!(error.contains("8"), "{error}");
}

#[test]
fn rejects_duplicate_names_in_the_input_header() {
  let image = build(&[
    ("icon", 0x10),
    ("icon", 0x20),
  ]);

  assert_eq!(
    N3dsExeFs.can_handle(&image).unwrap(),
    CodecHandlingConfidence::No
  );
  let error = N3dsExeFs
    .decode(&image, None)
    .err()
    .expect("duplicate ExeFS names cannot produce unique artifacts")
    .to_string();
  assert!(error.to_lowercase().contains("duplicate"), "{error}");
}

#[test]
fn refuses_duplicate_names_in_metadata() {
  let mut artifacts = decode(&build(&[
    ("icon", 0x100),
    ("banner", 0x100),
  ]));
  let mut metadata = ExefsMetadata::from_json(&artifacts[0].data).unwrap();
  metadata.files[1].name = metadata.files[0].name.clone();
  artifacts[0].data = metadata.to_json().unwrap();

  let Err(error) = N3dsExeFs.encode(Vec::new(), &artifacts, None) else {
    panic!("duplicate ExeFS metadata names must be rejected");
  };
  let error = error.to_string();
  assert!(error.to_lowercase().contains("duplicate"), "{error}");
}

#[test]
fn recomputes_hashes_when_a_file_changes() {
  let image = build(&[
    ("icon", 0x100),
    (".code", 0x400),
  ]);
  let mut artifacts = decode(&image);
  let icon = artifacts.iter_mut().find(|a| a.name == "icon").unwrap();
  icon.data.truncate(0x80);

  let rebuilt = N3dsExeFs.encode(Vec::new(), &artifacts, None).unwrap();
  let (files, _) = parse_file_headers(&rebuilt).unwrap();
  assert_eq!(files[1].offset % FILE_ALIGN, 0);
  assert_eq!(
    rebuilt[hash_slot(0)..hash_slot(0) + HASH_SIZE],
    sha256(&file_bytes(1, 0x80))
  );
}

#[test]
fn refuses_missing_or_extra_artifacts() {
  let image = build(&[
    ("icon", 0x100),
    (".code", 0x400),
  ]);
  let artifacts = decode(&image);
  let err = N3dsExeFs
    .encode(Vec::new(), &artifacts[..2], None)
    .expect_err("a missing file must fail")
    .to_string();
  assert!(err.contains(".code"), "{err}");

  let mut artifacts = decode(&image);
  artifacts.push(DecodedArtifact {
    name: "surprise".to_owned(),
    data: vec![0],
  });
  let err = N3dsExeFs
    .encode(Vec::new(), &artifacts, None)
    .expect_err("an unexpected file must fail")
    .to_string();
  assert!(err.contains("surprise"), "{err}");
}

#[test]
fn refuses_more_than_ten_files() {
  let image = build(&[("icon", 0x10)]);
  let artifacts = decode(&image);
  let mut metadata = ExefsMetadata::from_json(&artifacts[0].data).unwrap();
  metadata.files = (0..11)
    .map(|i| FileMetadata {
      name: format!("f{i}"),
      name_raw: None,
      offset: 0,
      gap: None,
    })
    .collect();
  let mut artifacts = artifacts;
  artifacts[0].data = metadata.to_json().unwrap();
  artifacts.truncate(1);
  for i in 0..11 {
    artifacts.push(DecodedArtifact {
      name: format!("f{i}"),
      data: vec![0],
    });
  }
  let err = N3dsExeFs
    .encode(Vec::new(), &artifacts, None)
    .expect_err("eleven files must fail")
    .to_string();
  assert!(err.contains("11"), "{err}");
}
