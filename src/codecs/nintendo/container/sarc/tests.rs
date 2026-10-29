use super::*;
use crate::codecs::nintendo::compression::lz11::Lz11;
use crate::codecs::{Codec, CodecHandlingConfidence, DecodedArtifact};

fn node(name: &str) -> NodeMetadata {
  NodeMetadata {
    name: Some(name.to_owned()),
    name_raw: None,
    artifact: None,
    hash: 0,
    attributes: HAS_NAME,
    name_offset: None,
    data_start: 0,
    data_end: 0,
  }
}

fn artifact(path: &str, data: &[u8]) -> DecodedArtifact {
  DecodedArtifact {
    name: format!("{FILES_PREFIX}{path}"),
    data: data.to_vec(),
  }
}

fn metadata_artifact(metadata: &SarcMetadata) -> DecodedArtifact {
  DecodedArtifact {
    name: METADATA_NAME.to_owned(),
    data: metadata.to_json().unwrap(),
  }
}

fn sample_metadata(endian: Endian) -> SarcMetadata {
  SarcMetadata {
    endian,
    header_size: u16::try_from(HEADER_SIZE).unwrap(),
    version: 0x0100,
    reserved: 0,
    data_offset: 0,
    header_extra: String::new(),
    hash_multiplier: 0x65,
    sfat_header_size: u16::try_from(SFAT_HEADER_SIZE).unwrap(),
    sfat_extra: String::new(),
    sfnt_header_size: u16::try_from(SFNT_HEADER_SIZE).unwrap(),
    sfnt_reserved: 0,
    sfnt_extra: String::new(),
    name_table: String::new(),
    data_size: 0,
    gaps: Vec::new(),
    trailing: String::new(),
    trailing_zeros: 0,
    nodes: vec![
      node("hello.bin"),
      node("data/world.bin"),
      node("data/empty"),
    ],
  }
}

fn sample_artifacts(endian: Endian) -> Vec<DecodedArtifact> {
  let metadata = sample_metadata(endian);
  vec![
    metadata_artifact(&metadata),
    artifact("hello.bin", b"hello"),
    artifact("data/world.bin", b"world world world"),
    artifact("data/empty", b""),
  ]
}

fn sample_source(endian: Endian) -> Vec<u8> {
  Sarc
    .encode(Vec::new(), &sample_artifacts(endian), None)
    .unwrap()
}

fn decode(source: &[u8]) -> Vec<DecodedArtifact> {
  Sarc.decode(source, None).unwrap()
}

fn encode(artifacts: &[DecodedArtifact]) -> Vec<u8> {
  Sarc.encode(Vec::new(), artifacts, None).unwrap()
}

fn decoded_metadata(artifacts: &[DecodedArtifact]) -> SarcMetadata {
  SarcMetadata::from_json(&artifacts[0].data).unwrap()
}

fn header_field(source: &[u8], offset: usize) -> u32 {
  Endian::Little.read_u32(source, offset).unwrap()
}

/// Returns the absolute offset of the first SFAT node.
fn nodes_start(source: &[u8]) -> usize {
  let header_size = Endian::Little.read_u16(source, 0x04).unwrap() as usize;
  let sfat_header_size = Endian::Little.read_u16(source, header_size + 0x04).unwrap() as usize;
  header_size + sfat_header_size
}

#[test]
fn round_trips_a_little_endian_archive() {
  let source = sample_source(Endian::Little);
  assert_eq!(&source[6..8], &[0xFF, 0xFE]);

  let artifacts = decode(&source);
  let mut names: Vec<&str> = artifacts.iter().map(|a| a.name.as_str()).collect();
  names.sort_unstable();
  assert_eq!(
    names,
    [
      "SARC.json",
      "files/data/empty",
      "files/data/world.bin",
      "files/hello.bin"
    ]
  );
  assert_eq!(
    artifacts
      .iter()
      .find(|artifact| artifact.name == "files/hello.bin")
      .unwrap()
      .data,
    b"hello"
  );
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn round_trips_a_big_endian_archive() {
  let source = sample_source(Endian::Big);
  assert_eq!(&source[6..8], &[0xFE, 0xFF]);
  let artifacts = decode(&source);
  assert_eq!(decoded_metadata(&artifacts).endian, Endian::Big);
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn lz11_sarc_preset_decodes_and_rebuilds_the_archive() {
  let source = sample_source(Endian::Little);
  let artifacts = decode(&source);
  let preset = Lz11.preset_args("sarc", None).unwrap();

  let compressed = Lz11.encode(Vec::new(), &artifacts, Some(&preset)).unwrap();
  let decoded = Lz11.decode(&compressed, Some(&preset)).unwrap();
  let decoded_contents: Vec<(&str, &[u8])> = decoded
    .iter()
    .map(|artifact| (artifact.name.as_str(), artifact.data.as_slice()))
    .collect();
  let source_contents: Vec<(&str, &[u8])> = artifacts
    .iter()
    .map(|artifact| (artifact.name.as_str(), artifact.data.as_slice()))
    .collect();
  assert_eq!(decoded_contents, source_contents);

  let rebuilt = Lz11.encode(Vec::new(), &decoded, Some(&preset)).unwrap();
  let raw = Lz11.decode(&rebuilt, None).unwrap();
  assert_eq!(raw[0].data, source);
}

#[test]
fn round_trips_an_empty_archive() {
  let mut metadata = sample_metadata(Endian::Little);
  metadata.nodes.clear();
  let artifacts = [metadata_artifact(
    &metadata,
  )];
  let source = encode(&artifacts);
  let decoded = decode(&source);
  assert_eq!(decoded.len(), 1);
  assert_eq!(decoded[0].name, "SARC.json");
  assert_eq!(encode(&decoded), source);
}

#[test]
fn computes_the_documented_hash() {
  assert_eq!(calc_hash(b"", 0x65), 0);
  assert_eq!(calc_hash(b"a", 0x65), 0x61);
  assert_eq!(calc_hash(b"ab", 0x65), 0x26A7);
  assert_eq!(calc_hash(b"abc", 0x65), 0xF4046);
}

#[test]
fn round_trips_unnamed_nodes() {
  let mut metadata = sample_metadata(Endian::Little);
  metadata.nodes.push(NodeMetadata {
    name: None,
    name_raw: None,
    artifact: Some("_sarc_unnamed_12345678".to_owned()),
    hash: 0x1234_5678,
    attributes: 0,
    name_offset: None,
    data_start: 0,
    data_end: 0,
  });
  let artifacts = vec![
    metadata_artifact(&metadata),
    artifact("hello.bin", b"hello"),
    artifact("data/world.bin", b"world world world"),
    artifact("data/empty", b""),
    artifact("_sarc_unnamed_12345678", b"nameless"),
  ];
  let source = encode(&artifacts);
  let decoded = decode(&source);
  let metadata = decoded_metadata(&decoded);
  let anonymous = metadata
    .nodes
    .iter()
    .find(|node| node.name.is_none())
    .unwrap();
  assert_eq!(anonymous.hash, 0x1234_5678);
  assert_eq!(
    anonymous.artifact.as_deref(),
    Some("_sarc_unnamed_12345678")
  );
  assert_eq!(encode(&decoded), source);
}

#[test]
fn preserves_unnamed_nodes_across_a_rename() {
  let mut metadata = sample_metadata(Endian::Little);
  metadata.nodes.push(NodeMetadata {
    name: None,
    name_raw: None,
    artifact: Some("_sarc_unnamed_0000abcd".to_owned()),
    hash: 0x0000_ABCD,
    attributes: 0,
    name_offset: None,
    data_start: 0,
    data_end: 0,
  });
  let artifacts = vec![
    metadata_artifact(&metadata),
    artifact("hello.bin", b"hello"),
    artifact("data/world.bin", b"world world world"),
    artifact("data/empty", b""),
    artifact("_sarc_unnamed_0000abcd", b"nameless"),
  ];
  let source = encode(&artifacts);
  let mut decoded = decode(&source);
  let mut metadata = decoded_metadata(&decoded);
  metadata
    .nodes
    .iter_mut()
    .find(|node| node.name.as_deref() == Some("hello.bin"))
    .unwrap()
    .name = Some("greeting.bin".to_owned());
  decoded[0].data = metadata.to_json().unwrap();
  let hello = decoded
    .iter_mut()
    .find(|artifact| artifact.name == "files/hello.bin")
    .unwrap();
  hello.name = "files/greeting.bin".to_owned();

  let rebuilt = encode(&decoded);
  let decoded = decode(&rebuilt);
  assert!(decoded.iter().any(|a| a.name == "files/greeting.bin"));
  assert!(
    decoded
      .iter()
      .any(|a| a.name == "files/_sarc_unnamed_0000abcd")
  );
}

#[test]
fn renames_an_entry_and_re_sorts_by_hash() {
  let source = sample_source(Endian::Little);
  let mut artifacts = decode(&source);
  let mut metadata = decoded_metadata(&artifacts);
  metadata
    .nodes
    .iter_mut()
    .find(|node| node.name.as_deref() == Some("hello.bin"))
    .unwrap()
    .name = Some("zzz.bin".to_owned());
  artifacts[0].data = metadata.to_json().unwrap();
  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "files/hello.bin")
    .unwrap()
    .name = "files/zzz.bin".to_owned();

  let rebuilt = encode(&artifacts);
  let decoded = decode(&rebuilt);
  let metadata = decoded_metadata(&decoded);
  let hashes: Vec<u32> = metadata.nodes.iter().map(|node| node.hash).collect();
  assert!(
    hashes.windows(2).all(|pair| pair[0] <= pair[1]),
    "hashes are not sorted: {hashes:?}"
  );
  for node in &metadata.nodes {
    if let Some(name) = &node.name {
      assert_eq!(
        node.hash,
        calc_hash(name.as_bytes(), metadata.hash_multiplier)
      );
    }
  }
  assert!(decoded.iter().any(|a| a.name == "files/zzz.bin"));
}

#[test]
fn preserves_the_upper_attribute_bits() {
  let mut metadata = sample_metadata(Endian::Little);
  metadata.nodes[0].attributes |= 0xAB00_0000;
  let artifacts = vec![
    metadata_artifact(&metadata),
    artifact("hello.bin", b"hello"),
    artifact("data/world.bin", b"world world world"),
    artifact("data/empty", b""),
  ];
  let source = encode(&artifacts);
  let mut decoded = decode(&source);
  let mut metadata = decoded_metadata(&decoded);
  metadata
    .nodes
    .iter_mut()
    .find(|node| node.name.as_deref() == Some("hello.bin"))
    .unwrap()
    .name = Some("renamed.bin".to_owned());
  decoded[0].data = metadata.to_json().unwrap();
  decoded
    .iter_mut()
    .find(|artifact| artifact.name == "files/hello.bin")
    .unwrap()
    .name = "files/renamed.bin".to_owned();

  let rebuilt = encode(&decoded);
  let decoded = decode(&rebuilt);
  let metadata = decoded_metadata(&decoded);
  let renamed = metadata
    .nodes
    .iter()
    .find(|node| node.name.as_deref() == Some("renamed.bin"))
    .unwrap();
  assert_eq!(renamed.attributes & 0xFF00_0000, 0xAB00_0000);
}

#[test]
fn preserves_non_zero_padding_as_gaps() {
  let mut source = sample_source(Endian::Little);
  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  let data_offset = metadata.data_offset as usize;
  let ranges: Vec<(usize, usize)> = metadata
    .nodes
    .iter()
    .filter(|node| node.data_end > node.data_start)
    .map(|node| (node.data_start as usize, node.data_end as usize))
    .collect();
  let relative = (0..metadata.data_size as usize)
    .find(|index| {
      !ranges
        .iter()
        .any(|&(start, end)| start <= *index && *index < end)
    })
    .expect("the synthetic archive has padding");
  let offset = data_offset + relative;
  assert_eq!(source[offset], 0);
  source[offset] = 0x5A;

  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  assert_ne!(metadata.gaps, Vec::<Gap>::new());
  let file_ranges: Vec<(u32, u32)> = metadata
    .nodes
    .iter()
    .filter(|node| node.data_end > node.data_start)
    .map(|node| {
      (
        metadata.data_offset + node.data_start,
        metadata.data_offset + node.data_end,
      )
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
  let mut source = sample_source(Endian::Little);
  source.extend_from_slice(b"tail");
  let artifacts = decode(&source);
  assert_eq!(decoded_metadata(&artifacts).trailing, hex::encode(b"tail"));
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn preserves_zero_trailing_padding() {
  let mut source = sample_source(Endian::Little);
  source.extend_from_slice(&[0; 0x77]);
  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  assert_eq!(metadata.trailing, String::new());
  assert_eq!(metadata.trailing_zeros, 0x77);
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn preserves_mixed_trailing_bytes() {
  let mut source = sample_source(Endian::Little);
  source.extend_from_slice(&[0, 0xAA, 0, 0]);
  let artifacts = decode(&source);
  let metadata = decoded_metadata(&artifacts);
  assert_eq!(metadata.trailing, hex::encode([0, 0xAA, 0, 0]));
  assert_eq!(metadata.trailing_zeros, 0);
  assert_eq!(encode(&artifacts), source);
}

#[test]
fn rejects_ambiguous_trailing_metadata() {
  let source = sample_source(Endian::Little);
  let mut artifacts = decode(&source);
  let mut metadata = decoded_metadata(&artifacts);
  metadata.trailing = "aa".to_owned();
  metadata.trailing_zeros = 1;
  artifacts[0] = metadata_artifact(&metadata);
  let error = Sarc.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(error.to_string().contains("both"), "{error}");
}

#[test]
fn recomputes_the_layout_when_a_file_grows() {
  let source = sample_source(Endian::Little);
  let mut artifacts = decode(&source);
  let hello = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "files/hello.bin")
    .unwrap();
  hello.data.extend_from_slice(b", world");

  let rebuilt = encode(&artifacts);
  let decoded = decode(&rebuilt);
  let hello = decoded
    .iter()
    .find(|artifact| artifact.name == "files/hello.bin")
    .unwrap();
  assert_eq!(hello.data, b"hello, world");
  assert_eq!(encode(&decoded), rebuilt);
}

#[test]
fn shifts_the_data_offset_for_a_longer_name_table() {
  let source = sample_source(Endian::Little);
  let mut artifacts = decode(&source);
  let mut metadata = decoded_metadata(&artifacts);
  let longer = "very/long/name/".repeat(16) + "hello.bin";
  metadata
    .nodes
    .iter_mut()
    .find(|node| node.name.as_deref() == Some("hello.bin"))
    .unwrap()
    .name = Some(longer.clone());
  artifacts[0].data = metadata.to_json().unwrap();
  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "files/hello.bin")
    .unwrap()
    .name = format!("files/{longer}");

  let rebuilt = encode(&artifacts);
  let decoded = decode(&rebuilt);
  assert!(
    decoded
      .iter()
      .any(|artifact| artifact.name == format!("files/{longer}"))
  );
  assert_eq!(encode(&decoded), rebuilt);
}

#[test]
fn detects_valid_archives() {
  let source = sample_source(Endian::Little);
  assert_eq!(
    Sarc.can_handle(&source).unwrap(),
    CodecHandlingConfidence::Certain
  );
  assert_eq!(
    Sarc.can_handle(b"not a sarc archive").unwrap(),
    CodecHandlingConfidence::No
  );
  assert_eq!(
    Sarc.can_handle(&source[..10]).unwrap(),
    CodecHandlingConfidence::No
  );
}

#[test]
fn rejects_malformed_headers() {
  let source = sample_source(Endian::Little);

  let mut bad_magic = source.clone();
  bad_magic[..4].copy_from_slice(b"XARC");
  assert!(Sarc.decode(&bad_magic, None).is_err());

  let mut bad_bom = source.clone();
  bad_bom[6..8].copy_from_slice(&[0, 0]);
  assert!(Sarc.decode(&bad_bom, None).is_err());

  assert!(Sarc.decode(&source[..HEADER_SIZE - 1], None).is_err());

  let sfat_offset = Endian::Little.read_u16(&source, 0x04).unwrap() as usize;
  let mut bad_sfat = source.clone();
  bad_sfat[sfat_offset..sfat_offset + 4].copy_from_slice(b"XFAT");
  assert!(Sarc.decode(&bad_sfat, None).is_err());

  let sfnt_offset = nodes_start(&source) + 3 * NODE_SIZE;
  let mut bad_sfnt = source.clone();
  bad_sfnt[sfnt_offset..sfnt_offset + 4].copy_from_slice(b"XFNT");
  assert!(Sarc.decode(&bad_sfnt, None).is_err());

  let mut bad_data_offset = source;
  bad_data_offset[0x0C..0x10].copy_from_slice(&Endian::Little.u32_bytes(0));
  assert!(Sarc.decode(&bad_data_offset, None).is_err());
}

#[test]
fn rejects_invalid_node_ranges() {
  let source = sample_source(Endian::Little);
  let nodes = nodes_start(&source);

  let mut backwards = source.clone();
  backwards[nodes + 0x08..nodes + 0x0C].copy_from_slice(&Endian::Little.u32_bytes(100));
  backwards[nodes + 0x0C..nodes + 0x10].copy_from_slice(&Endian::Little.u32_bytes(50));
  assert!(Sarc.decode(&backwards, None).is_err());

  let mut too_far = source.clone();
  let total = header_field(&source, 0x08);
  let data_offset = header_field(&source, 0x0C);
  too_far[nodes + 0x0C..nodes + 0x10]
    .copy_from_slice(&Endian::Little.u32_bytes(total - data_offset + 1));
  assert!(Sarc.decode(&too_far, None).is_err());

  let mut overlap = source.clone();
  let non_empty: Vec<usize> = (0..3)
    .filter(|index| {
      let base = nodes + index * NODE_SIZE;
      header_field(&source, base + 0x0C) > header_field(&source, base + 0x08)
    })
    .collect();
  let (first, second) = (non_empty[0], non_empty[1]);
  let first_start = header_field(&source, nodes + first * NODE_SIZE + 0x08);
  let first_end = header_field(&source, nodes + first * NODE_SIZE + 0x0C);
  let second_base = nodes + second * NODE_SIZE;
  overlap[second_base + 0x08..second_base + 0x0C]
    .copy_from_slice(&Endian::Little.u32_bytes(first_start));
  overlap[second_base + 0x0C..second_base + 0x10]
    .copy_from_slice(&Endian::Little.u32_bytes(first_end));
  assert!(Sarc.decode(&overlap, None).is_err());
}

#[test]
fn rejects_a_bad_name_offset() {
  let source = sample_source(Endian::Little);
  let nodes = nodes_start(&source);
  let mut bad = source;
  bad[nodes + 0x04..nodes + 0x08]
    .copy_from_slice(&Endian::Little.u32_bytes(HAS_NAME | 0x00FF_FFFF));
  assert!(Sarc.decode(&bad, None).is_err());
}

#[test]
fn rejects_broken_metadata() {
  let source = sample_source(Endian::Little);
  let artifacts = decode(&source);

  let error = Sarc.encode(Vec::new(), &[], None).unwrap_err();
  assert!(error.to_string().contains("found none"), "{error}");

  let mut metadata = decoded_metadata(&artifacts);
  metadata.nodes[0].name = Some(String::new());
  let broken = vec![metadata_artifact(
    &metadata,
  )];
  let error = Sarc.encode(Vec::new(), &broken, None).unwrap_err();
  assert!(error.to_string().contains("empty node name"), "{error}");

  let mut metadata = decoded_metadata(&artifacts);
  metadata.nodes[0].name = Some("/absolute".to_owned());
  let broken = vec![metadata_artifact(
    &metadata,
  )];
  let error = Sarc.encode(Vec::new(), &broken, None).unwrap_err();
  assert!(error.to_string().contains("must not start"), "{error}");

  let mut metadata = decoded_metadata(&artifacts);
  metadata.nodes[0].name = Some("a//b".to_owned());
  let broken = vec![metadata_artifact(
    &metadata,
  )];
  let error = Sarc.encode(Vec::new(), &broken, None).unwrap_err();
  assert!(error.to_string().contains("empty, '.' or '..'"), "{error}");

  let mut metadata = decoded_metadata(&artifacts);
  metadata.nodes.push(NodeMetadata {
    name: None,
    name_raw: None,
    artifact: None,
    hash: 0,
    attributes: 0,
    name_offset: None,
    data_start: 0,
    data_end: 0,
  });
  let mut with_files = decode(&source);
  with_files[0].data = metadata.to_json().unwrap();
  let error = Sarc.encode(Vec::new(), &with_files, None).unwrap_err();
  assert!(
    error.to_string().contains("without an artifact name"),
    "{error}"
  );
}

#[test]
fn rejects_missing_and_unexpected_artifacts() {
  let source = sample_source(Endian::Little);
  let mut artifacts = decode(&source);

  let index = artifacts
    .iter()
    .position(|artifact| artifact.name == "files/hello.bin")
    .unwrap();
  let removed = artifacts.remove(index);
  let error = Sarc.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(
    error
      .to_string()
      .contains("Expected a files/hello.bin artifact"),
    "{error}"
  );
  artifacts.insert(index, removed);

  artifacts.push(DecodedArtifact {
    name: "files/extra".to_owned(),
    data: vec![1],
  });
  let error = Sarc.encode(Vec::new(), &artifacts, None).unwrap_err();
  assert!(error.to_string().contains("Unexpected artifact"), "{error}");
}
