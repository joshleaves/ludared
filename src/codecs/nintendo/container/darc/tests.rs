use super::*;
use crate::codecs::nintendo::compression::lz10::Lz10;
use crate::codecs::{Codec, CodecHandlingConfidence, DecodedArtifact};

fn entry(kind: EntryKind, parent: &str, name: &str) -> EntryMetadata {
  EntryMetadata {
    kind,
    parent: parent.to_owned(),
    name: name.to_owned(),
    name_raw: None,
    name_offset: None,
    data_offset: None,
    size: None,
    data_alignment: None,
  }
}

fn file(parent: &str, name: &str, data: &[u8]) -> DecodedArtifact {
  DecodedArtifact {
    name: format!(
      "{FILES_PREFIX}{}{name}",
      if parent.is_empty() {
        String::new()
      } else {
        format!("{parent}/")
      }
    ),
    data: data.to_vec(),
  }
}

fn metadata_artifact(metadata: &DarcMetadata) -> DecodedArtifact {
  DecodedArtifact {
    name: METADATA_NAME.to_owned(),
    data: metadata.to_json().unwrap(),
  }
}

fn sample_metadata(endian: Endian, dot_entry: bool) -> DarcMetadata {
  DarcMetadata {
    endian,
    header_size: 0x1C,
    version: 0x0001_0000,
    table_offset: 0x1C,
    data_offset: 0,
    prefix: String::new(),
    name_area: String::new(),
    table_padding: String::new(),
    data_size: 0,
    gaps: Vec::new(),
    trailing: String::new(),
    trailing_zeros: 0,
    dot_entry,
    root_name_offset: 0,
    dot_name_offset: None,
    entries: vec![
      entry(EntryKind::File, "", "hello.bin"),
      entry(EntryKind::Folder, "", "data"),
      entry(EntryKind::File, "data", "plain.bin"),
      entry(EntryKind::File, "data", "empty"),
    ],
  }
}

fn sample_artifacts(endian: Endian, dot_entry: bool) -> Vec<DecodedArtifact> {
  let metadata = sample_metadata(endian, dot_entry);
  vec![
    metadata_artifact(&metadata),
    file("", "hello.bin", b"hello"),
    file("data", "plain.bin", b"a plain file"),
    file("data", "empty", b""),
  ]
}

fn sample_source(endian: Endian, dot_entry: bool) -> Vec<u8> {
  Darc
    .encode(Vec::new(), &sample_artifacts(endian, dot_entry), None)
    .unwrap()
}

fn decode(source: &[u8]) -> Vec<DecodedArtifact> {
  Darc.decode(source, None).unwrap()
}

fn encode(artifacts: &[DecodedArtifact]) -> Vec<u8> {
  Darc.encode(Vec::new(), artifacts, None).unwrap()
}

fn decoded_metadata(artifacts: &[DecodedArtifact]) -> DarcMetadata {
  DarcMetadata::from_json(&artifacts[0].data).unwrap()
}

fn file_offset(source: &[u8], index: usize) -> u32 {
  let table_offset = Endian::Little.read_u32(source, 0x10).unwrap() as usize;
  Endian::Little
    .read_u32(source, table_offset + index * ENTRY_SIZE + 4)
    .unwrap()
}

fn source_with_extra_gap() -> Vec<u8> {
  let mut artifacts = decode(&sample_source(Endian::Little, false));
  let mut metadata = decoded_metadata(&artifacts);
  for entry in &mut metadata.entries {
    if entry.name != "hello.bin"
      && let Some(offset) = entry.data_offset.as_mut()
    {
      *offset += 16;
    }
  }
  metadata.data_size += 16;
  artifacts[0] = metadata_artifact(&metadata);
  encode(&artifacts)
}

#[test]
fn round_trips_a_little_endian_archive() {
  let source = sample_source(Endian::Little, false);
  assert_eq!(&source[4..6], &[0xFF, 0xFE]);

  let artifacts = decode(&source);
  let names: Vec<&str> = artifacts.iter().map(|a| a.name.as_str()).collect();
  assert_eq!(
    names,
    [
      "DARC.json",
      "files/hello.bin",
      "files/data/plain.bin",
      "files/data/empty"
    ]
  );
  assert_eq!(artifacts[1].data, b"hello");
  assert_eq!(artifacts[2].data, b"a plain file");
  assert_eq!(artifacts[3].data, [0u8; 0]);

  assert_eq!(encode(&artifacts), source);
}

#[test]
fn round_trips_a_big_endian_archive() {
  let source = sample_source(Endian::Big, false);
  assert_eq!(&source[4..6], &[0xFE, 0xFF]);
  let artifacts = decode(&source);
  assert_eq!(decoded_metadata(&artifacts).endian, Endian::Big);
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn round_trips_the_dot_entry() {
  let source = sample_source(Endian::Little, true);
  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  assert!(metadata.dot_entry);
  assert!(metadata.dot_name_offset.is_some());
  // The entries at the top level hang off the "." entry, not the synthetic
  // root, so the folder at index 3 points at index 1.
  let table_offset = Endian::Little.read_u32(&source, 0x10).unwrap() as usize;
  let parent = Endian::Little
    .read_u32(&source, table_offset + 3 * ENTRY_SIZE + 0x04)
    .unwrap();
  assert_eq!(parent, 1);
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn round_trips_an_empty_folder() {
  let mut metadata = sample_metadata(Endian::Little, false);
  metadata.entries.push(entry(EntryKind::Folder, "", "timg"));
  let mut artifacts = sample_artifacts(Endian::Little, false);
  artifacts[0] = metadata_artifact(&metadata);

  let source = encode(&artifacts);
  let decoded = decode(&source);
  let metadata = decoded_metadata(&decoded);
  let folder = metadata.entries.last().unwrap();
  assert!(matches!(folder.kind, EntryKind::Folder));
  assert_eq!(folder.parent, "");
  assert_eq!(folder.name, "timg");
  assert_eq!(encode(&decoded), source);
}

#[test]
fn lz10_darc_preset_decodes_and_rebuilds_the_archive() {
  let source = sample_source(Endian::Little, false);
  let artifacts = decode(&source);
  let preset = Lz10.preset_args("darc", None).unwrap();

  let compressed = Lz10.encode(Vec::new(), &artifacts, Some(&preset)).unwrap();
  let decoded = Lz10.decode(&compressed, Some(&preset)).unwrap();
  let decoded_contents: Vec<(&str, &[u8])> = decoded
    .iter()
    .map(|artifact| (artifact.name.as_str(), artifact.data.as_slice()))
    .collect();
  let source_contents: Vec<(&str, &[u8])> = artifacts
    .iter()
    .map(|artifact| (artifact.name.as_str(), artifact.data.as_slice()))
    .collect();
  assert_eq!(decoded_contents, source_contents);

  let rebuilt = Lz10.encode(Vec::new(), &decoded, Some(&preset)).unwrap();
  let raw = Lz10.decode(&rebuilt, None).unwrap();
  assert_eq!(raw[0].data, source);
}

/// Pins the real Nintendo parent indices and nested paths in a third-party
/// archive, including the `.` entry used as the top-level parent.
#[test]
fn reads_the_dot_entry_and_nested_folder_parents() {
  let archive = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/testing/fixtures/nintendo/nintendo_darc.arc"
  ));
  let second_word = |index: usize| {
    let base = 0x1C + index * ENTRY_SIZE + 4;
    u32::from_le_bytes(archive[base..base + 4].try_into().unwrap())
  };

  assert_eq!((second_word(1), second_word(3), second_word(5)), (0, 1, 3));

  let artifacts = Darc.decode(archive, None).unwrap();
  let metadata: serde_json::Value = serde_json::from_slice(&artifacts[0].data).unwrap();
  assert_eq!(metadata["dot_entry"], serde_json::json!(true));
  let entries = metadata["entries"].as_array().unwrap();
  let tree: Vec<(&str, &str)> = entries
    .iter()
    .map(|entry| {
      (
        entry["name"].as_str().unwrap(),
        entry["parent"].as_str().unwrap(),
      )
    })
    .collect();
  assert_eq!(
    tree,
    vec![
      ("hi.txt", ""),
      ("data", ""),
      ("pattern.bin", "data"),
      ("nested", "data"),
      ("noise.bin", "data/nested"),
    ]
  );
}

#[test]
fn round_trips_an_archive_with_only_a_root() {
  let metadata = DarcMetadata {
    endian: Endian::Little,
    header_size: 0x1C,
    version: 0x0001_0000,
    table_offset: 0x1C,
    data_offset: 0,
    prefix: String::new(),
    name_area: String::new(),
    table_padding: String::new(),
    data_size: 0,
    gaps: Vec::new(),
    trailing: String::new(),
    trailing_zeros: 0,
    dot_entry: false,
    root_name_offset: 0,
    dot_name_offset: None,
    entries: Vec::new(),
  };
  let artifacts = [metadata_artifact(
    &metadata,
  )];
  let source = encode(&artifacts);
  let decoded = decode(&source);
  assert_eq!(decoded.len(), 1);
  assert_eq!(decoded[0].name, "DARC.json");
  assert_eq!(encode(&decoded), source);
}

#[test]
fn preserves_raw_utf16_names() {
  // An unpaired high surrogate cannot be represented as UTF-8.
  let mut metadata = sample_metadata(Endian::Little, false);
  metadata.entries.push(EntryMetadata {
    kind: EntryKind::File,
    parent: String::new(),
    name: "_darc_raw_00d8".to_owned(),
    name_raw: Some("00d8".to_owned()),
    name_offset: None,
    data_offset: None,
    size: None,
    data_alignment: None,
  });
  let artifacts = vec![
    metadata_artifact(&metadata),
    file("", "hello.bin", b"hello"),
    file("data", "plain.bin", b"a plain file"),
    file("data", "empty", b""),
    file("", "_darc_raw_00d8", b"raw"),
  ];
  let source = encode(&artifacts);
  let decoded = decode(&source);
  let metadata = decoded_metadata(&decoded);
  let raw = metadata
    .entries
    .iter()
    .find(|entry| entry.name_raw.is_some())
    .unwrap();
  assert_eq!(raw.name, "_darc_raw_00d8");
  assert_eq!(raw.name_raw.as_deref(), Some("00d8"));
  assert_eq!(encode(&decoded), source);
}

#[test]
fn preserves_non_zero_padding_as_gaps() {
  let mut source = sample_source(Endian::Little, false);
  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  let hello = metadata
    .entries
    .iter()
    .find(|entry| entry.name == "hello.bin")
    .unwrap();
  let offset = hello.data_offset.unwrap() as usize + hello.size.unwrap() as usize;
  // The next file starts on a 0x20-byte boundary, so this byte is padding.
  assert_eq!(source[offset], 0);
  source[offset] = 0xA5;

  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  assert!(
    metadata.gaps.iter().any(|gap| gap.offset as usize <= offset
      && offset < gap.offset as usize + gap.data.len() / 2),
    "gaps: {:?}",
    metadata.gaps
  );
  let file_ranges: Vec<(u32, u32)> = metadata
    .entries
    .iter()
    .filter(|entry| entry.kind == EntryKind::File && entry.size.unwrap_or(0) > 0)
    .map(|entry| {
      let start = entry.data_offset.unwrap();
      (start, start + entry.size.unwrap())
    })
    .collect();
  for gap in &metadata.gaps {
    let gap_end = gap.offset + u32::try_from(gap.data.len()).unwrap() / 2;
    for &(start, end) in &file_ranges {
      assert!(
        gap.offset >= end || gap_end <= start,
        "gap {:#x}..{:#x} overlaps file data {start:#x}..{end:#x}",
        gap.offset,
        gap_end
      );
    }
  }
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn preserves_trailing_bytes() {
  let mut source = sample_source(Endian::Little, false);
  source.extend_from_slice(b"trailing bytes");
  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  assert_eq!(metadata.trailing, hex::encode(b"trailing bytes"));
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn preserves_zero_trailing_padding() {
  let mut source = sample_source(Endian::Little, false);
  source.extend_from_slice(&[0; 0x77]);
  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  assert_eq!(metadata.trailing, String::new());
  assert_eq!(metadata.trailing_zeros, 0x77);
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn preserves_mixed_trailing_bytes() {
  let mut source = sample_source(Endian::Little, false);
  source.extend_from_slice(&[0, 0xAA, 0, 0]);
  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  assert_eq!(metadata.trailing, hex::encode([0, 0xAA, 0, 0]));
  assert_eq!(metadata.trailing_zeros, 0);
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn rejects_ambiguous_trailing_metadata() {
  let source = sample_source(Endian::Little, false);
  let mut artifacts = decode(&source);
  let mut metadata = decoded_metadata(&artifacts);
  metadata.trailing = "aa".to_owned();
  metadata.trailing_zeros = 1;
  artifacts[0] = metadata_artifact(&metadata);
  let error = Darc.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(error.to_string().contains("both"), "{error}");
}

#[test]
fn recomputes_the_layout_when_a_file_grows() {
  let source = sample_source(Endian::Little, false);
  let mut artifacts = decode(&source);
  let hello = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "files/hello.bin")
    .unwrap();
  hello.data.extend_from_slice(b" and more");

  let rebuilt = encode(&artifacts);
  let decoded = decode(&rebuilt);
  let hello = decoded
    .iter()
    .find(|artifact| artifact.name == "files/hello.bin")
    .unwrap();
  assert_eq!(hello.data, b"hello and more");
  assert_eq!(encode(&decoded), rebuilt);
}

#[test]
fn grows_into_available_zero_padding_without_moving_files() {
  let source = source_with_extra_gap();
  let mut artifacts = decode(&source);
  let before: Vec<u32> = (1..=3).map(|index| file_offset(&source, index)).collect();
  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "files/hello.bin")
    .unwrap()
    .data
    .extend_from_slice(b"12345678");

  let rebuilt = encode(&artifacts);
  let after: Vec<u32> = (1..=3).map(|index| file_offset(&rebuilt, index)).collect();
  assert_eq!(after, before);
  assert_eq!(decode(&rebuilt)[1].data, b"hello12345678");
}

#[test]
fn same_size_replacement_keeps_the_original_offset() {
  let source = sample_source(Endian::Little, false);
  let mut artifacts = decode(&source);
  let original_offset = file_offset(&source, 1);
  artifacts[1].data.copy_from_slice(b"HELLO");

  let rebuilt = encode(&artifacts);
  assert_eq!(file_offset(&rebuilt, 1), original_offset);
  assert_eq!(decode(&rebuilt)[1].data, b"HELLO");
}

#[test]
fn shrinking_a_file_keeps_unrelated_offsets() {
  let source = sample_source(Endian::Little, false);
  let mut artifacts = decode(&source);
  let plain_offset = file_offset(&source, 3);
  artifacts[1].data.truncate(2);

  let rebuilt = encode(&artifacts);
  assert_eq!(file_offset(&rebuilt, 3), plain_offset);
  assert_eq!(decode(&rebuilt)[1].data, b"he");
}

#[test]
fn handles_multiple_file_size_changes_in_one_relayout() {
  let source = sample_source(Endian::Little, false);
  let mut artifacts = decode(&source);
  artifacts[1].data.extend_from_slice(&[0xAA; 96]);
  artifacts[2].data.extend_from_slice(b"tail");

  let rebuilt = encode(&artifacts);
  let decoded = decode(&rebuilt);
  assert_eq!(
    decoded[1].data,
    [
      b"hello".as_slice(),
      &[0xAA; 96]
    ]
    .concat()
  );
  assert_eq!(decoded[2].data, b"a plain filetail");
  assert_eq!(encode(&decoded), rebuilt);
}

#[test]
fn preserves_nonzero_padding_when_relayout_is_required() {
  let mut artifacts = decode(&source_with_extra_gap());
  let mut metadata = decoded_metadata(&artifacts);
  let empty = metadata
    .entries
    .iter_mut()
    .find(|entry| entry.name == "empty")
    .unwrap();
  *empty.data_offset.as_mut().unwrap() += 32;
  metadata.data_size += 32;
  artifacts[0] = metadata_artifact(&metadata);
  let mut source = encode(&artifacts);
  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  let plain = metadata
    .entries
    .iter()
    .find(|entry| entry.name == "plain.bin")
    .unwrap();
  let pad_offset = plain.data_offset.unwrap() as usize + plain.size.unwrap() as usize + 8;
  source[pad_offset] = 0xA5;
  let mut artifacts = decode(&source);
  let mut metadata = decoded_metadata(&artifacts);
  let hello = metadata
    .entries
    .iter_mut()
    .find(|entry| entry.name == "hello.bin")
    .unwrap();
  hello.name = "a-much-longer-name-that-forces-a-table-relayout.bin".to_owned();
  hello.name_offset = None;
  artifacts[1].name = format!("files/{}", hello.name);
  artifacts[0] = metadata_artifact(&metadata);

  let rebuilt = encode(&artifacts);
  let rebuilt_artifacts = decode(&rebuilt);
  let rebuilt_metadata = decoded_metadata(&rebuilt_artifacts);
  assert!(
    rebuilt_metadata
      .gaps
      .iter()
      .any(|gap| gap.data.contains("a5"))
  );
  assert_eq!(encode(&rebuilt_artifacts), rebuilt);
}

#[test]
fn infers_repeated_mixed_alignment_constraints_on_relayout() {
  let mut metadata = sample_metadata(Endian::Little, false);
  metadata.entries = vec![
    entry(EntryKind::File, "", "base"),
    entry(EntryKind::File, "", "aligned-a"),
    entry(EntryKind::File, "", "aligned-b"),
    entry(EntryKind::File, "", "aligned-c"),
    entry(EntryKind::File, "", "plain"),
  ];
  for entry in &mut metadata.entries[1..4] {
    entry.data_alignment = Some(0x80);
  }
  let artifacts = vec![
    metadata_artifact(&metadata),
    file("", "base", b"base"),
    file("", "aligned-a", b"a"),
    file("", "aligned-b", b"b"),
    file("", "aligned-c", b"c"),
    file("", "plain", b"p"),
  ];
  let source = encode(&artifacts);
  let decoded = decode(&source);
  let source_metadata = decoded_metadata(&decoded);
  for name in [
    "aligned-a",
    "aligned-b",
    "aligned-c",
  ] {
    assert_eq!(
      source_metadata
        .entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap()
        .data_alignment,
      Some(0x80)
    );
  }

  let mut changed = decoded;
  changed
    .iter_mut()
    .find(|artifact| artifact.name == "files/base")
    .unwrap()
    .data
    .extend_from_slice(&[0; 0x200]);
  let rebuilt = encode(&changed);
  let rebuilt_metadata = decoded_metadata(&decode(&rebuilt));
  for name in [
    "aligned-a",
    "aligned-b",
    "aligned-c",
  ] {
    let entry = rebuilt_metadata
      .entries
      .iter()
      .find(|entry| entry.name == name)
      .unwrap();
    assert_eq!(entry.data_offset.unwrap() % 0x80, 0);
  }
  let plain = rebuilt_metadata
    .entries
    .iter()
    .find(|entry| entry.name == "plain")
    .unwrap();
  assert_eq!(plain.data_offset.unwrap() % 4, 0);
}

#[test]
fn ignores_a_single_coincidental_high_alignment_gap() {
  let entries = vec![
    EntryMetadata {
      data_offset: Some(0x100),
      size: Some(1),
      ..entry(EntryKind::File, "", "a")
    },
    EntryMetadata {
      data_offset: Some(0x180),
      size: Some(1),
      ..entry(EntryKind::File, "", "b")
    },
  ];
  assert_eq!(infer_data_alignments(&entries, 0x100), vec![None, None]);
}

#[test]
fn renames_an_entry_and_rebuilds_the_names() {
  let source = sample_source(Endian::Little, false);
  let mut artifacts = decode(&source);
  let mut metadata = decoded_metadata(&artifacts);
  let hello = metadata
    .entries
    .iter_mut()
    .find(|entry| entry.name == "hello.bin")
    .unwrap();
  hello.name = "greeting.bin".to_owned();
  hello.name_raw = None;
  hello.name_offset = None;
  artifacts[0].data = metadata.to_json().unwrap();
  artifacts[1].name = "files/greeting.bin".to_owned();

  let rebuilt = encode(&artifacts);
  let decoded = decode(&rebuilt);
  let names: Vec<&str> = decoded.iter().map(|a| a.name.as_str()).collect();
  assert_eq!(
    names,
    [
      "DARC.json",
      "files/greeting.bin",
      "files/data/plain.bin",
      "files/data/empty"
    ]
  );
}

#[test]
fn removes_an_entry_and_resizes_the_table_padding() {
  let source = sample_source(Endian::Little, false);
  let mut artifacts = decode(&source);
  let mut metadata = decoded_metadata(&artifacts);

  // Give the table non-zero padding, then move the data area past it.
  let table_end =
    metadata.table_offset as usize + Endian::Little.read_u32(&source, 0x14).unwrap() as usize;
  metadata.data_offset += 0x10;
  let padding_len = metadata.data_offset as usize - table_end;
  metadata.table_padding = hex::encode(vec![0xA5; padding_len]);
  for entry in &mut metadata.entries {
    if let Some(offset) = entry.data_offset.as_mut() {
      *offset += 0x10;
    }
  }
  artifacts[0].data = metadata.to_json().unwrap();
  let padded = encode(&artifacts);
  assert_eq!(encode(&decode(&padded)), padded);

  // Removing an entry shrinks the table, so the recorded padding no longer fits.
  let mut artifacts = decode(&padded);
  let mut metadata = decoded_metadata(&artifacts);
  assert_ne!(metadata.table_padding, "");
  metadata.entries.retain(|entry| entry.name != "empty");
  artifacts[0].data = metadata.to_json().unwrap();
  artifacts.retain(|artifact| artifact.name != "files/data/empty");

  let rebuilt = encode(&artifacts);
  let decoded = decode(&rebuilt);
  let names: Vec<&str> = decoded.iter().map(|a| a.name.as_str()).collect();
  assert_eq!(
    names,
    [
      "DARC.json",
      "files/hello.bin",
      "files/data/plain.bin"
    ]
  );
  assert_eq!(encode(&decoded), rebuilt);
}

#[test]
fn detects_valid_archives() {
  let source = sample_source(Endian::Little, false);
  assert_eq!(
    Darc.can_handle(&source).unwrap(),
    CodecHandlingConfidence::Certain
  );
  assert_eq!(
    Darc.can_handle(b"not a darc archive").unwrap(),
    CodecHandlingConfidence::No
  );
  assert_eq!(
    Darc.can_handle(&source[..10]).unwrap(),
    CodecHandlingConfidence::No
  );
}

#[test]
fn rejects_malformed_headers() {
  let source = sample_source(Endian::Little, false);

  let mut bad_magic = source.clone();
  bad_magic[..4].copy_from_slice(b"DARC");
  assert!(Darc.decode(&bad_magic, None).is_err());

  let mut bad_bom = source.clone();
  bad_bom[4..6].copy_from_slice(&[0, 0]);
  assert!(Darc.decode(&bad_bom, None).is_err());

  assert!(Darc.decode(&source[..HEADER_SIZE - 1], None).is_err());

  let mut bad_size = source.clone();
  let declared = Endian::Little.read_u32(&source, 0x0C).unwrap();
  bad_size[0x0C..0x10].copy_from_slice(&Endian::Little.u32_bytes(declared + 1));
  assert!(Darc.decode(&bad_size, None).is_err());
}

#[test]
fn rejects_a_non_folder_root() {
  let mut source = sample_source(Endian::Little, false);
  let table_offset = Endian::Little.read_u32(&source, 0x10).unwrap() as usize;
  let name_field = Endian::Little.read_u32(&source, table_offset).unwrap();
  source[table_offset..table_offset + 4]
    .copy_from_slice(&Endian::Little.u32_bytes(name_field & !FOLDER_FLAG));
  assert!(Darc.decode(&source, None).is_err());
}

#[test]
fn rejects_broken_metadata() {
  let source = sample_source(Endian::Little, false);
  let artifacts = decode(&source);

  let error = Darc
    .encode(
      Vec::new(),
      &[DecodedArtifact {
        name: "not-darc.json".to_owned(),
        data: vec![],
      }],
      None,
    )
    .unwrap_err();
  assert!(error.to_string().contains("DARC.json"), "{error}");

  let error = Darc.encode(Vec::new(), &[], None).unwrap_err();
  assert!(error.to_string().contains("found none"), "{error}");

  let mut metadata = decoded_metadata(&artifacts);
  metadata.entries[1].parent = "missing".to_owned();
  let broken = vec![metadata_artifact(
    &metadata,
  )];
  let error = Darc.encode(Vec::new(), &broken, None).unwrap_err();
  assert!(error.to_string().contains("unknown parent"), "{error}");

  let mut metadata = decoded_metadata(&artifacts);
  metadata.entries[2].name = "hello.bin".to_owned();
  metadata.entries[2].parent = String::new();
  let broken = vec![metadata_artifact(
    &metadata,
  )];
  let error = Darc.encode(Vec::new(), &broken, None).unwrap_err();
  assert!(
    error.to_string().contains("listed before its parent") || error.to_string().contains("twice"),
    "{error}"
  );
}

#[test]
fn rejects_missing_and_unexpected_artifacts() {
  let source = sample_source(Endian::Little, false);
  let mut artifacts = decode(&source);

  let removed = artifacts.remove(1);
  let error = Darc.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(
    error
      .to_string()
      .contains("Expected a files/hello.bin artifact"),
    "{error}"
  );
  artifacts.insert(1, removed);

  artifacts.push(DecodedArtifact {
    name: "files/extra".to_owned(),
    data: vec![1],
  });
  let error = Darc.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(error.to_string().contains("Unexpected artifact"), "{error}");
}
