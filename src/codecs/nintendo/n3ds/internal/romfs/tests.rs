use super::*;
use crate::codecs::{Codec, CodecHandlingConfidence, DecodedArtifact};

fn sample_metadata() -> RomfsMetadata {
  RomfsMetadata {
    magic_id: 0x10000,
    master_hash_size: 0, // recomputed on encode
    optional_info_size: IVFC_HEADER_SIZE as u32,
    reserved_tail: hex::encode([0; RESERVED_TAIL_SIZE]),
    levels: [
      LevelMetadata {
        logical_offset: 0,
        hash_data_size: 0,
        block_size_log2: 0xC,
        reserved: 0,
      },
      LevelMetadata {
        logical_offset: 0,
        hash_data_size: 0,
        block_size_log2: 0xC,
        reserved: 0,
      },
      LevelMetadata {
        logical_offset: 0,
        hash_data_size: 0,
        block_size_log2: 0xC,
        reserved: 0,
      },
    ],
    dir_hash_buckets: Some(hash_table_count(3)),
    file_hash_buckets: Some(hash_table_count(3)),
    directories: vec![
      DirMetadata {
        parent: String::new(),
        name: String::new(),
        name_raw: None,
      },
      DirMetadata {
        parent: String::new(),
        name: "text".to_owned(),
        name_raw: None,
      },
    ],
    files: vec![
      FileMetadata {
        parent: String::new(),
        name: "hello.txt".to_owned(),
        name_raw: None,
        data_offset: 0,
        size: 0x21,
      },
      FileMetadata {
        parent: "text".to_owned(),
        name: "chapter1.txt".to_owned(),
        name_raw: None,
        data_offset: 0x30,
        size: 0x11,
      },
      FileMetadata {
        parent: "text".to_owned(),
        name: "empty".to_owned(),
        name_raw: None,
        data_offset: 0,
        size: 0,
      },
    ],
  }
}

fn sample_artifacts() -> Vec<DecodedArtifact> {
  let metadata = sample_metadata();
  let mut artifacts = vec![DecodedArtifact {
    name: METADATA_NAME.to_owned(),
    data: metadata.to_json().unwrap(),
  }];
  artifacts.push(DecodedArtifact {
    name: "files/hello.txt".to_owned(),
    data: (0..0x21u8).collect(),
  });
  artifacts.push(DecodedArtifact {
    name: "files/text/chapter1.txt".to_owned(),
    data: vec![0xAB; 0x11],
  });
  artifacts.push(DecodedArtifact {
    name: "files/text/empty".to_owned(),
    data: Vec::new(),
  });
  artifacts
}

fn encode(artifacts: &[DecodedArtifact]) -> Vec<u8> {
  N3dsRomFs.encode(Vec::new(), artifacts, None).unwrap()
}

fn decode(image: &[u8]) -> Vec<DecodedArtifact> {
  N3dsRomFs.decode(image, None).unwrap()
}

fn two_root_file_image() -> Vec<u8> {
  let mut metadata = sample_metadata();
  metadata.directories.truncate(1);
  metadata.files = ["alpha", "bravo"]
    .into_iter()
    .map(|name| FileMetadata {
      parent: String::new(),
      name: name.to_owned(),
      name_raw: None,
      data_offset: 0,
      size: 8,
    })
    .collect();
  let artifacts = vec![
    DecodedArtifact {
      name: METADATA_NAME.to_owned(),
      data: metadata.to_json().unwrap(),
    },
    DecodedArtifact {
      name: "files/alpha".to_owned(),
      data: b"alpha---".to_vec(),
    },
    DecodedArtifact {
      name: "files/bravo".to_owned(),
      data: b"bravo---".to_vec(),
    },
  ];
  encode(&artifacts)
}

fn overwrite_file_name(image: &mut [u8], entry_index: usize, name: &[u16]) {
  let level3_range = parse_ivfc(image).unwrap().level(2).unwrap();
  let level3_start = level3_range.start;
  let level3 = &image[level3_range];
  let mut entry_offset = u32_le(level3, 0x1C).unwrap() as usize;
  for _ in 0..entry_index {
    let name_size = u32_le(level3, entry_offset + 0x1C).unwrap() as usize;
    entry_offset += file_entry_size(name_size) as usize;
  }
  let name_size = u32_le(level3, entry_offset + 0x1C).unwrap() as usize;
  assert_eq!(
    name_size,
    name.len() * 2,
    "test name must preserve table layout"
  );
  let start = level3_start + entry_offset + FILE_ENTRY_FIXED;
  for (index, unit) in name.iter().enumerate() {
    image[start + index * 2..start + index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
  }
}

#[test]
fn round_trips_a_built_image() {
  let image = encode(&sample_artifacts());
  let header = parse_ivfc(&image).unwrap();
  let physical_level3 = header.level(2).unwrap();
  assert_eq!(physical_level3.start, 0x1000);
  assert_eq!(header.levels[2].logical_offset, 0x2000);
  assert_ne!(
    physical_level3.start as u64,
    header.levels[2].logical_offset
  );
  assert_eq!(
    N3dsRomFs.can_handle(&image).unwrap(),
    CodecHandlingConfidence::Certain
  );
  let artifacts = decode(&image);
  let names: Vec<&str> = artifacts.iter().map(|a| a.name.as_str()).collect();
  assert_eq!(
    names,
    [
      METADATA_NAME,
      "files/hello.txt",
      "files/text/chapter1.txt",
      "files/text/empty"
    ]
  );
  assert_eq!(artifacts[1].data, (0..0x21u8).collect::<Vec<u8>>());
  assert_eq!(artifacts[3].data, Vec::<u8>::new());
  assert_eq!(encode(&artifacts), image);
}

#[test]
fn rebuilds_after_edits() {
  let image = encode(&sample_artifacts());
  let mut artifacts = decode(&image);
  let chapter = artifacts
    .iter_mut()
    .find(|a| a.name == "files/text/chapter1.txt")
    .unwrap();
  chapter.data = vec![0xCD; 0x40];

  let rebuilt = encode(&artifacts);
  assert_eq!(
    N3dsRomFs.can_handle(&rebuilt).unwrap(),
    CodecHandlingConfidence::Certain
  );
  let artifacts = decode(&rebuilt);
  assert_eq!(artifacts[2].data, vec![0xCD; 0x40]);
  assert_eq!(encode(&artifacts), rebuilt);
}

#[test]
fn rejects_foreign_data() {
  for image in [
    Vec::new(),
    vec![0; 0x2000],
    vec![0xFF; 0x2000],
  ] {
    assert_eq!(
      N3dsRomFs.can_handle(&image).unwrap(),
      CodecHandlingConfidence::No
    );
    assert!(N3dsRomFs.decode(&image, None).is_err());
  }
}

#[test]
fn refuses_missing_or_extra_artifacts() {
  let artifacts = sample_artifacts();
  let err = N3dsRomFs
    .encode(Vec::new(), &artifacts[..2], None)
    .expect_err("a missing file must fail")
    .to_string();
  assert!(err.contains("files/text/chapter1.txt"), "{err}");

  let mut artifacts = sample_artifacts();
  artifacts.push(DecodedArtifact {
    name: "files/surprise".to_owned(),
    data: vec![0],
  });
  let err = N3dsRomFs
    .encode(Vec::new(), &artifacts, None)
    .expect_err("an unexpected file must fail")
    .to_string();
  assert!(err.contains("surprise"), "{err}");
}

#[test]
fn refuses_a_tree_without_a_root() {
  let mut artifacts = sample_artifacts();
  let mut metadata = sample_metadata();
  metadata.directories.remove(0);
  artifacts[0].data = metadata.to_json().unwrap();
  let err = N3dsRomFs
    .encode(Vec::new(), &artifacts, None)
    .expect_err("a rootless tree must fail")
    .to_string();
  assert!(err.contains("root"), "{err}");
}

#[test]
fn rejects_invalid_ivfc_block_sizes() {
  let mut image = encode(&sample_artifacts());
  image[0x4C..0x50].copy_from_slice(&u32::MAX.to_le_bytes());

  assert_eq!(
    N3dsRomFs.can_handle(&image).unwrap(),
    CodecHandlingConfidence::No
  );
  assert!(N3dsRomFs.decode(&image, None).is_err());
}

#[test]
fn rejects_directory_that_points_to_a_later_parent() {
  let mut image = encode(&sample_artifacts());
  let level3_range = parse_ivfc(&image).unwrap().level(2).unwrap();
  let level3_start = level3_range.start;
  let dir_meta_offset = u32_le(&image[level3_range.clone()], 0xC).unwrap() as usize;
  let first_dir_offset = level3_start + dir_meta_offset;
  // The second directory's parent field must reference the root (offset 0),
  // not its own offset, which creates a forward/self parent link.
  let second_dir_offset = first_dir_offset + dir_entry_size(0) as usize;
  let self_parent = u32::try_from(dir_entry_size(0)).unwrap();
  image[second_dir_offset..second_dir_offset + 4].copy_from_slice(&self_parent.to_le_bytes());

  let error = N3dsRomFs
    .decode(&image, None)
    .err()
    .expect("a directory cannot reference itself as a parent")
    .to_string();
  assert!(error.contains("not earlier"), "{error}");
}

#[test]
fn rejects_file_names_that_escape_or_break_artifact_paths() {
  let mut image = two_root_file_image();
  overwrite_file_name(
    &mut image,
    0,
    &[
      b'a' as u16,
      b'/' as u16,
      b'p' as u16,
      b'h' as u16,
      b'a' as u16,
    ],
  );

  let error = N3dsRomFs
    .decode(&image, None)
    .err()
    .expect("a slash in a RomFS name cannot become an artifact path")
    .to_string();
  assert!(error.contains("path component"), "{error}");
}

#[test]
fn rejects_overlapping_file_and_directory_artifact_paths() {
  let mut metadata = sample_metadata();
  metadata.files = vec![
    FileMetadata {
      parent: String::new(),
      name: "text".to_owned(),
      name_raw: None,
      data_offset: 0,
      size: 8,
    },
    FileMetadata {
      parent: "text".to_owned(),
      name: "chapter.txt".to_owned(),
      name_raw: None,
      data_offset: 0x10,
      size: 8,
    },
  ];
  let collision = vec![
    DecodedArtifact {
      name: METADATA_NAME.to_owned(),
      data: metadata.to_json().unwrap(),
    },
    DecodedArtifact {
      name: "files/text".to_owned(),
      data: b"file----".to_vec(),
    },
    DecodedArtifact {
      name: "files/text/chapter.txt".to_owned(),
      data: b"chapter-".to_vec(),
    },
  ];
  let Err(error) = N3dsRomFs.encode(Vec::new(), &collision, None) else {
    panic!("a file path cannot also be a parent of another path");
  };
  let error = error.to_string();
  assert!(error.contains("overlap"), "{error}");
}

#[test]
fn rejects_duplicate_file_artifact_paths() {
  let mut image = two_root_file_image();
  overwrite_file_name(
    &mut image,
    1,
    &[
      b'a' as u16,
      b'l' as u16,
      b'p' as u16,
      b'h' as u16,
      b'a' as u16,
    ],
  );

  let error = N3dsRomFs
    .decode(&image, None)
    .err()
    .expect("duplicate RomFS files cannot become unique artifacts")
    .to_string();
  assert!(error.contains("duplicate"), "{error}");
}

#[test]
fn preserves_invalid_utf16_names_with_safe_artifact_paths() {
  let mut image = two_root_file_image();
  overwrite_file_name(
    &mut image,
    1,
    &[
      0xD800,
      b'r' as u16,
      b'a' as u16,
      b'v' as u16,
      b'o' as u16,
    ],
  );

  let artifacts = decode(&image);
  assert_eq!(artifacts[2].name, "files/_romfs_raw_00d87200610076006f00");
  let metadata = RomfsMetadata::from_json(&artifacts[0].data).unwrap();
  assert_eq!(
    metadata.files[1].name_raw.as_deref(),
    Some("00d87200610076006f00")
  );

  let rebuilt = encode(&artifacts);
  let rebuilt_artifacts = decode(&rebuilt);
  assert_eq!(rebuilt_artifacts[2].name, artifacts[2].name);
  assert_eq!(rebuilt_artifacts[2].data, artifacts[2].data);
}

#[test]
fn rejects_file_data_ranges_past_level3() {
  let mut image = encode(&sample_artifacts());
  let level3_start = parse_ivfc(&image).unwrap().level(2).unwrap().start;
  let level3 = &image[level3_start..];
  let file_meta_offset = u32::from_le_bytes(level3[0x1C..0x20].try_into().unwrap()) as usize;
  let file_data_offset = u32::from_le_bytes(level3[0x24..0x28].try_into().unwrap());
  let level3_length = level3.len() as u64;
  let first_file_offset = level3_start + file_meta_offset + 0x8;
  image[first_file_offset..first_file_offset + 8]
    .copy_from_slice(&(level3_length + 1).to_le_bytes());

  let err = match N3dsRomFs.decode(&image, None) {
    Ok(_) => panic!("file data outside level 3 must be rejected"),
    Err(err) => err.to_string(),
  };
  assert!(
    err.contains("data offset overflows") || err.contains("data range exceeds"),
    "{err}"
  );
  assert!(file_data_offset > file_meta_offset as u32);
}

#[test]
fn rejects_malformed_level3_table_layout() {
  let mut image = encode(&sample_artifacts());
  let level3 = parse_ivfc(&image).unwrap().level(2).unwrap().start;
  image[level3 + 0x4..level3 + 0x8].copy_from_slice(&0u32.to_le_bytes());

  assert!(N3dsRomFs.decode(&image, None).is_err());
}

#[test]
fn preserves_all_three_ivfc_reserved_words() {
  let mut image = encode(&sample_artifacts());
  image[0x20..0x24].copy_from_slice(&0x1122_3344u32.to_le_bytes());
  image[0x38..0x3C].copy_from_slice(&0x5566_7788u32.to_le_bytes());
  image[0x50..0x54].copy_from_slice(&0x99AA_BBCCu32.to_le_bytes());

  let rebuilt = encode(&decode(&image));
  assert_eq!(rebuilt, image);
}

#[test]
fn hash_bucket_counts_follow_the_makerom_rule() {
  assert_eq!(hash_table_count(0), 3);
  assert_eq!(hash_table_count(2), 3);
  assert_eq!(hash_table_count(4), 5);
  assert_eq!(hash_table_count(18), 19);
  assert_eq!(hash_table_count(20), 23);
}
