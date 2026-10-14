use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::DecodedArtifact;
use crate::codecs::cdrom::iso9660::Iso9660;
use hadris_iso::file::EntryType;
use hadris_iso::read::{IsoImage, PathSeparator};
use hadris_iso::write::options::{CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
use std::io::Cursor;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{IsoMetadata, METADATA_NAME};

fn metadata(artifacts: &[DecodedArtifact]) -> IsoMetadata {
  IsoMetadata::from_json(&artifacts[0].data).unwrap()
}

/// A copy of the artifacts, which `DecodedArtifact` cannot make of itself.
fn copy(artifacts: &[DecodedArtifact]) -> Vec<DecodedArtifact> {
  artifacts
    .iter()
    .map(|artifact| DecodedArtifact {
      name: artifact.name.clone(),
      data: artifact.data.clone(),
    })
    .collect()
}

/// Replaces the metadata of a decode, leaving every file artifact alone.
fn with_metadata(artifacts: &[DecodedArtifact], metadata: &IsoMetadata) -> Vec<DecodedArtifact> {
  let mut artifacts = copy(artifacts);
  artifacts[0].data = metadata.to_json().unwrap();

  artifacts
}

fn build(volume_identifier: &str, files: Vec<IsoFile>) -> Vec<u8> {
  let options = IsoFormatOptions {
    volume_name: volume_identifier.to_owned(),
    system_id: None,
    volume_set_id: None,
    publisher_id: None,
    preparer_id: None,
    application_id: None,
    sector_size: 2048,
    path_separator: PathSeparator::ForwardSlash,
    features: CreationFeatures::default(),
    strict_charset: false,
  };

  let mut buffer = Cursor::new(Vec::new());
  IsoImageWriter::create(
    &mut buffer,
    InputFiles {
      path_separator: PathSeparator::ForwardSlash,
      files,
    },
    options,
  )
  .unwrap();

  buffer.into_inner()
}

/// An ISO built the way the codec builds one.
fn image(files: Vec<IsoFile>) -> Vec<u8> {
  build("LUDARED", files)
}

fn file(name: &str, contents: &[u8]) -> IsoFile {
  IsoFile::File {
    name: Arc::new(name.to_owned()),
    contents: contents.to_vec(),
  }
}

fn directory(name: &str, children: Vec<IsoFile>) -> IsoFile {
  IsoFile::Directory {
    name: Arc::new(name.to_owned()),
    children,
  }
}

fn artifact(name: &str, data: &[u8]) -> DecodedArtifact {
  DecodedArtifact {
    name: name.to_owned(),
    data: data.to_vec(),
  }
}

/// The names a decode produced, metadata first.
fn names(artifacts: &[DecodedArtifact]) -> Vec<&str> {
  artifacts
    .iter()
    .map(|artifact| artifact.name.as_str())
    .collect()
}

/// The image's primary directory tree, which every read here goes through.
fn plain_root(image: &IsoImage<Cursor<Vec<u8>>>) -> hadris_iso::read::RootDir {
  image
    .root_dirs()
    .iter()
    .find(|root| matches!(root.entry_type(), EntryType::Level1 { .. }))
    .copied()
    .expect("the image should carry a primary directory tree")
}

/// Every file the image holds, read through its plain ISO 9660 namespace: the
/// path, the contents, and the sector it sits at.
///
/// Identifiers are taken from the records themselves, and `keep_version` decides
/// whether the `;1` comes off them.
fn walk_image(bytes: &[u8], keep_version: bool) -> Vec<(String, Vec<u8>, u32)> {
  let image = IsoImage::open(Cursor::new(bytes.to_vec())).unwrap();

  let mut found = Vec::new();
  let mut pending = vec![(plain_root(&image).iter(&image), String::new())];
  let mut visited = 0;

  while let Some((directory, prefix)) = pending.pop() {
    visited += 1;
    assert!(visited < 4096, "the tree does not end");

    for entry in directory.entries() {
      let entry = entry.unwrap();
      if entry.is_special() {
        continue;
      }

      let raw = entry.record.name();
      let raw = match raw.iter().position(|byte| *byte == b';') {
        Some(index) if !keep_version => &raw[..index],
        _ => raw,
      };

      let name = String::from_utf8_lossy(raw).into_owned();
      let path = if prefix.is_empty() {
        name
      } else {
        format!("{prefix}/{name}")
      };

      if entry.is_directory() {
        pending.push((image.open_dir(entry.as_dir_ref(&image).unwrap()), path));
        continue;
      }

      found.push((
        path,
        image.read_file(&entry).unwrap(),
        entry.header().extent.read(),
      ));
    }
  }

  found
}

/// Every file the image holds, by path.
fn read_back(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
  let mut found: Vec<(String, Vec<u8>)> = walk_image(bytes, false)
    .into_iter()
    .map(|(path, data, _)| (path, data))
    .collect();

  found.sort();
  found
}

/// Every file the image holds, in the order they sit on the disc.
fn on_disc(bytes: &[u8]) -> Vec<String> {
  let mut found = walk_image(bytes, false);

  found.sort_by_key(|(_, _, extent)| *extent);
  found.into_iter().map(|(path, _, _)| path).collect()
}

/// Every file identifier the image holds, `;1` and all.
fn identifiers(bytes: &[u8]) -> Vec<String> {
  let mut found: Vec<String> = walk_image(bytes, true)
    .into_iter()
    .map(|(path, _, _)| path)
    .collect();

  found.sort();
  found
}

/// An ISO built by `mkisofs`, so that at least some of what these tests read was
/// not something Hadris wrote.
///
/// `extra` goes to cdrtools verbatim, so a test can ask for the extensions this
/// codec deliberately leaves out. `None` where cdrtools is not installed, so
/// that the tests using it skip rather than fail on a machine without it.
fn mkisofs(volume_identifier: &str, files: &[(&str, &[u8])], extra: &[&str]) -> Option<Vec<u8>> {
  static COUNTER: AtomicUsize = AtomicUsize::new(0);

  Command::new("mkisofs").arg("--version").output().ok()?;

  let base = std::env::temp_dir().join(format!(
    "ludared-iso9660-{}-{}",
    std::process::id(),
    COUNTER.fetch_add(1, Ordering::Relaxed)
  ));
  let source = base.join("src");

  for (name, data) in files {
    let path = source.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, data).unwrap();
  }

  let image = base.join("out.iso");
  let built = Command::new("mkisofs")
    .arg("-quiet")
    .arg("-iso-level")
    .arg("1")
    .arg(format!("-V{volume_identifier}"))
    .args(extra)
    .arg("-o")
    .arg(&image)
    .arg(&source)
    .status()
    .unwrap();

  let built = built.success().then(|| std::fs::read(&image).unwrap());
  std::fs::remove_dir_all(&base).ok();

  built
}

/// The image `mkisofs -R -J` produces: Rock Ridge on the primary descriptor and
/// a Joliet supplementary descriptor beside it.
fn mkisofs_both_extensions(volume_identifier: &str, files: &[(&str, &[u8])]) -> Option<Vec<u8>> {
  mkisofs(volume_identifier, files, &["-R", "-J"])
}

/// Three files across two directories, laid out in an order that is neither
/// alphabetical nor the order the directories list their entries in.
fn sample() -> Vec<u8> {
  build(
    "LUDARED",
    vec![
      file("Z.BIN", b"z"),
      directory(
        "A",
        vec![file(
          "FIRST.BIN",
          b"first",
        )],
      ),
      file("M.BIN", b"m"),
      directory(
        "DIR",
        vec![file(
          "INNER.BIN",
          b"inner",
        )],
      ),
    ],
  )
}

/// A rebuild of a single file, named `name`.
fn encode_one(name: &str) -> Result<Vec<u8>, crate::codecs::errors::CodecError> {
  let artifacts = [
    artifact(
      METADATA_NAME,
      &serde_json::to_vec(&IsoMetadata {
        volume_identifier: "TEST".to_owned(),
        file_order: vec![name.to_owned()],
      })
      .unwrap(),
    ),
    artifact(name, b"x"),
  ];

  Iso9660.encode(Vec::new(), &artifacts, None)
}

#[test]
fn recognizes_an_iso_9660_image() {
  assert_eq!(
    Iso9660.can_handle(&sample()),
    CodecHandlingConfidence::Certain
  );
}

#[test]
fn refuses_input_that_is_not_an_iso_9660_image() {
  for data in [
    vec![],
    vec![0u8; 16 * 2048],
    {
      let mut bytes = sample();
      bytes[16 * 2048] = 0x02; // a supplementary, not a primary, descriptor
      bytes
    },
    {
      let mut bytes = sample();
      bytes[16 * 2048 + 1] = b'X'; // `CD001` no longer identifies itself
      bytes
    },
  ] {
    assert_eq!(
      Iso9660.can_handle(&data),
      CodecHandlingConfidence::No,
      "should not have been recognized"
    );
  }
}

#[test]
fn extracts_every_file_with_its_directory_structure() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  // In the order the files sit on the disc. Hadris lays a directory's own
  // files down before those of the directories it holds, so the two at the
  // root precede the two below.
  assert_eq!(
    names(&artifacts)[1..],
    [
      "Z.BIN",
      "M.BIN",
      "A/FIRST.BIN",
      "DIR/INNER.BIN"
    ]
  );

  assert_eq!(
    read_back(&sample()),
    [
      ("A/FIRST.BIN".to_owned(), b"first".to_vec()),
      ("DIR/INNER.BIN".to_owned(), b"inner".to_vec()),
      ("M.BIN".to_owned(), b"m".to_vec()),
      ("Z.BIN".to_owned(), b"z".to_vec()),
    ]
  );
}

#[test]
fn puts_the_metadata_first() {
  for image in [
    sample(),
    image(vec![file(
      "ONLY.BIN",
      b"x",
    )]),
    image(vec![
      file("Z.BIN", b"1"),
      file("A.BIN", b"2"),
    ]),
  ] {
    let artifacts = Iso9660.decode(&image, None).unwrap();

    assert_eq!(
      artifacts[0].name, METADATA_NAME,
      "the metadata should lead, under its own name"
    );
    assert!(
      IsoMetadata::from_json(&artifacts[0].data).is_ok(),
      "the first artifact should hold the metadata"
    );
  }
}

#[test]
fn keeps_the_volume_identifier_it_was_given() {
  for volume in [
    "SAKURA_TAISEN",
    "MY GAME",
    "A",
    &"X".repeat(32),
  ] {
    let source = build(volume, vec![file("F.BIN", b"x")]);
    let artifacts = Iso9660.decode(&source, None).unwrap();

    assert_eq!(
      metadata(&artifacts).volume_identifier,
      volume,
      "the identifier should survive the decode"
    );
  }
}

#[test]
fn keeps_a_volume_identifier_the_charset_would_rather_not_write() {
  // `strict_charset: true` would fold this into `MY_SATURN_DISC`, so the exact
  // value is what the round trip has to keep.
  let source = build("My Saturn Disc", vec![file("F.BIN", b"x")]);
  let rebuilt = Iso9660
    .encode(Vec::new(), &Iso9660.decode(&source, None).unwrap(), None)
    .unwrap();

  assert_eq!(
    metadata(&Iso9660.decode(&rebuilt, None).unwrap()).volume_identifier,
    "My Saturn Disc",
    "the identifier should survive decode, encode and decode as it stands"
  );
}

#[test]
fn orders_the_metadata_by_where_the_files_sit() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  // Directory listings are alphabetical, so A/FIRST.BIN would come before
  // Z.BIN were the metadata recording that instead of the layout.
  assert_eq!(metadata(&artifacts).file_order, on_disc(&source));
  assert_eq!(metadata(&artifacts).file_order[0], "Z.BIN");
}

#[test]
fn writes_the_metadata_as_readable_json() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();
  let json = String::from_utf8(artifacts[0].data.clone()).unwrap();

  assert_eq!(
    json,
    r#"{
  "volume_identifier": "LUDARED",
  "file_order": [
    "Z.BIN",
    "M.BIN",
    "A/FIRST.BIN",
    "DIR/INNER.BIN"
  ]
}"#
  );
}

#[test]
fn rebuilds_an_image_holding_the_same_files() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();
  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  assert_eq!(
    read_back(&rebuilt),
    read_back(&source),
    "the rebuilt image should hold the same files"
  );

  // The point of the metadata: the files come out in the order they went in.
  // Where they land on the disc is not promised, only which order they follow.
  assert_eq!(on_disc(&rebuilt), on_disc(&source));
}

#[test]
fn carries_the_volume_identifier_through_a_rebuild() {
  let source = build("SAKURA_TAISEN", vec![file("F.BIN", b"x")]);
  let artifacts = Iso9660.decode(&source, None).unwrap();
  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  let again = Iso9660.decode(&rebuilt, None).unwrap();

  assert_eq!(metadata(&again).volume_identifier, "SAKURA_TAISEN");
}

#[test]
fn rebuilds_naming_the_volume_the_metadata_records() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  let edited = with_metadata(
    &artifacts,
    &IsoMetadata {
      volume_identifier: "RENAMED_VOLUME".to_owned(),
      file_order: metadata(&artifacts).file_order,
    },
  );

  let rebuilt = Iso9660.encode(Vec::new(), &edited, None).unwrap();
  let again = Iso9660.decode(&rebuilt, None).unwrap();

  assert_eq!(metadata(&again).volume_identifier, "RENAMED_VOLUME");
}

#[test]
fn rebuilds_after_a_file_changes_size() {
  let source = sample();
  let mut artifacts = Iso9660.decode(&source, None).unwrap();

  for (name, data) in [
    ("Z.BIN", vec![b'z'; 9000]),
    ("A/FIRST.BIN", b"first".to_vec()),
    ("M.BIN", b"m".to_vec()),
  ] {
    artifacts
      .iter_mut()
      .find(|artifact| artifact.name == name)
      .unwrap()
      .data = data;
  }

  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();
  let contents = read_back(&rebuilt);

  assert_eq!(contents[0].0, "A/FIRST.BIN");
  assert_eq!(contents[2], ("M.BIN".to_owned(), b"m".to_vec()));
  assert_eq!(contents[3], ("Z.BIN".to_owned(), vec![b'z'; 9000]));
}

#[test]
fn rebuilds_after_a_file_shrinks_to_nothing() {
  let source = image(vec![
    file("KEEP.BIN", b"keep"),
    file("DROP.BIN", b"drop"),
  ]);
  let mut artifacts = Iso9660.decode(&source, None).unwrap();

  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "DROP.BIN")
    .unwrap()
    .data
    .clear();

  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  assert_eq!(
    read_back(&rebuilt),
    [
      ("DROP.BIN".to_owned(), Vec::new()),
      ("KEEP.BIN".to_owned(), b"keep".to_vec()),
    ]
  );
}

#[test]
fn rebuilds_after_a_file_grows_from_nothing() {
  let source = image(vec![
    file("EMPTY.BIN", b""),
    file("FULL.BIN", b"full"),
  ]);
  let mut artifacts = Iso9660.decode(&source, None).unwrap();

  artifacts
    .iter_mut()
    .find(|artifact| artifact.name == "EMPTY.BIN")
    .unwrap()
    .data = b"now it has something in it".to_vec();

  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  assert_eq!(
    read_back(&rebuilt),
    [
      (
        "EMPTY.BIN".to_owned(),
        b"now it has something in it".to_vec()
      ),
      ("FULL.BIN".to_owned(), b"full".to_vec()),
    ]
  );
}

#[test]
fn rebuilds_a_file_added_to_the_metadata_and_the_artifacts() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  // The file has to be named in the metadata as well as handed over: the
  // metadata is what says where it goes.
  let mut meta = metadata(&artifacts);
  meta.file_order.push("EXTRA.BIN".to_owned());

  let mut extended = copy(&artifacts);
  extended.push(artifact("EXTRA.BIN", b"extra"));
  extended[0].data = meta.to_json().unwrap();

  let rebuilt = Iso9660.encode(Vec::new(), &extended, None).unwrap();
  let contents = read_back(&rebuilt);
  let paths: Vec<&str> = contents.iter().map(|(path, _)| path.as_str()).collect();

  assert!(
    paths.contains(&"EXTRA.BIN"),
    "the added file should be in the image: {paths:?}"
  );
  assert_eq!(contents.len(), read_back(&source).len() + 1);
}

#[test]
fn refuses_a_file_the_metadata_does_not_name() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  let mut extended = copy(&artifacts);
  extended.push(artifact("EXTRA.BIN", b"extra"));

  let err = Iso9660.encode(Vec::new(), &extended, None).unwrap_err();

  assert!(
    err.to_string().contains("EXTRA.BIN"),
    "the unplaced file should be named: {err}"
  );
}

#[test]
fn refuses_a_file_the_metadata_names_that_is_not_there() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  let mut meta = metadata(&artifacts);
  meta.file_order.push("MISSING.BIN".to_owned());

  let err = Iso9660
    .encode(Vec::new(), &with_metadata(&artifacts, &meta), None)
    .unwrap_err();

  assert!(
    err.to_string().contains("MISSING.BIN"),
    "the missing file should be named: {err}"
  );
}

#[test]
fn refuses_a_file_named_by_the_metadata_twice() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  let mut meta = metadata(&artifacts);
  meta.file_order.push("Z.BIN".to_owned());

  let err = Iso9660
    .encode(Vec::new(), &with_metadata(&artifacts, &meta), None)
    .unwrap_err();

  assert!(
    err.to_string().contains("more than once"),
    "the repetition should be reported: {err}"
  );
}

#[test]
fn lays_the_files_out_in_the_order_the_metadata_gives() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let mut meta = metadata(&artifacts);
  meta.file_order = vec![
    "M.BIN".to_owned(),
    "DIR/INNER.BIN".to_owned(),
    "A/FIRST.BIN".to_owned(),
    "Z.BIN".to_owned(),
  ];

  let rebuilt = Iso9660
    .encode(Vec::new(), &with_metadata(&artifacts, &meta), None)
    .unwrap();

  // A directory's own files still come before those of the directories it
  // holds, so the two at the root lead whatever the metadata asked for.
  assert_eq!(
    on_disc(&rebuilt),
    [
      "M.BIN",
      "Z.BIN",
      "DIR/INNER.BIN",
      "A/FIRST.BIN"
    ]
  );
}

#[test]
fn never_puts_the_metadata_inside_the_image() {
  let rebuilt = Iso9660
    .encode(Vec::new(), &Iso9660.decode(&sample(), None).unwrap(), None)
    .unwrap();

  let rebuilt_files = read_back(&rebuilt);
  let paths: Vec<String> = rebuilt_files.into_iter().map(|(path, _)| path).collect();

  assert!(
    !paths.iter().any(|path| path.ends_with(".json")),
    "the metadata was laid into the image: {paths:?}"
  );
  assert_eq!(
    Iso9660.decode(&rebuilt, None).unwrap()[0].name,
    METADATA_NAME,
    "the rebuilt image should hold exactly one metadata, its own"
  );
}

#[test]
fn takes_the_metadata_off_the_front_whatever_it_is_named() {
  let mut renamed = copy(&Iso9660.decode(&sample(), None).unwrap());
  renamed[0].name = "ANYTHING_AT_ALL".to_owned();

  let rebuilt = Iso9660.encode(Vec::new(), &renamed, None).unwrap();

  // Position is what identifies the metadata, so the name it happens to carry
  // says nothing about where it goes or what the volume ends up called.
  assert_eq!(read_back(&rebuilt), read_back(&sample()));

  assert_eq!(
    metadata(&Iso9660.decode(&rebuilt, None).unwrap()).volume_identifier,
    "LUDARED",
    "the filename should have had no say in the volume identifier"
  );
}

#[test]
fn refuses_artifacts_with_no_metadata() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  assert!(Iso9660.encode(Vec::new(), &artifacts[1..], None).is_err());
  assert!(Iso9660.encode(Vec::new(), &[], None).is_err());
}

#[test]
fn refuses_artifacts_of_the_same_name() {
  let artifacts = [
    artifact(
      METADATA_NAME,
      br#"{"volume_identifier":"X","file_order":["A","A"]}"#,
    ),
    artifact("A.BIN", b"one"),
    artifact("A.BIN", b"two"),
  ];

  assert!(Iso9660.encode(Vec::new(), &artifacts, None).is_err());
}

#[test]
fn refuses_metadata_that_is_not_the_shape_it_expects() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  for json in [
    &b"not json at all"[..],
    b"[]",
    b"{}",
    br#"{"volume_identifier":"X"}"#,
    br#"{"file_order":[]}"#,
    br#"{"volume_identifier":42,"file_order":[]}"#,
    br#"{"volume_identifier":"X","file_order":"A"}"#,
  ] {
    let mut broken = copy(&artifacts);
    broken[0].data = json.to_vec();

    let err = Iso9660.encode(Vec::new(), &broken, None).unwrap_err();

    assert!(
      err.to_string().contains("ISO metadata"),
      "'{}' should be refused: {err}",
      String::from_utf8_lossy(json)
    );
  }
}

#[test]
fn keeps_nested_directories_and_duplicate_basenames_apart() {
  let source = image(vec![
    directory(
      "Z",
      vec![file(
        "DATA.BIN",
        b"z data",
      )],
    ),
    directory(
      "A",
      vec![file(
        "DATA.BIN",
        b"a data",
      )],
    ),
    directory(
      "M",
      vec![directory(
        "INNER",
        vec![file(
          "DATA.BIN",
          b"m data",
        )],
      )],
    ),
  ]);

  let artifacts = Iso9660.decode(&source, None).unwrap();
  assert_eq!(
    names(&artifacts)[1..],
    [
      "Z/DATA.BIN",
      "A/DATA.BIN",
      "M/INNER/DATA.BIN"
    ]
  );

  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  assert_eq!(
    read_back(&rebuilt),
    [
      ("A/DATA.BIN".to_owned(), b"a data".to_vec()),
      ("M/INNER/DATA.BIN".to_owned(), b"m data".to_vec()),
      ("Z/DATA.BIN".to_owned(), b"z data".to_vec()),
    ]
  );
}

#[test]
fn keeps_directory_paths_from_splitting_into_several_directories() {
  // One `M`, shared, rather than two directories of that name side by side.
  let source = image(vec![directory(
    "M",
    vec![
      directory(
        "INNER",
        vec![file(
          "ONE.BIN",
          b"1",
        )],
      ),
      directory(
        "DEEP",
        vec![file(
          "TWO.BIN",
          b"2",
        )],
      ),
    ],
  )]);

  let artifacts = Iso9660.decode(&source, None).unwrap();
  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  assert_eq!(
    read_back(&rebuilt),
    [
      ("M/DEEP/TWO.BIN".to_owned(), b"2".to_vec()),
      ("M/INNER/ONE.BIN".to_owned(), b"1".to_vec()),
    ]
  );
}

#[test]
fn decodes_an_image_holding_no_files_at_all() {
  let source = image(vec![directory(
    "EMPTY",
    vec![],
  )]);
  let artifacts = Iso9660.decode(&source, None).unwrap();

  assert_eq!(names(&artifacts), [METADATA_NAME]);
  assert!(metadata(&artifacts).file_order.is_empty());

  assert!(Iso9660.encode(Vec::new(), &artifacts, None).is_ok());
}

#[test]
fn refuses_input_holding_no_volume_descriptor() {
  assert!(Iso9660.decode(&vec![0u8; 16 * 2048], None).is_err());
  assert!(Iso9660.decode(&[], None).is_err());
}

#[test]
fn writes_an_image_with_neither_rock_ridge_nor_joliet() {
  let rebuilt = Iso9660
    .encode(Vec::new(), &Iso9660.decode(&sample(), None).unwrap(), None)
    .unwrap();

  let image = IsoImage::open(Cursor::new(rebuilt)).unwrap();

  assert!(
    !image.supports_rrip(),
    "the rebuild should carry no Rock Ridge"
  );

  let roots: Vec<EntryType> = image
    .root_dirs()
    .iter()
    .map(|root| root.entry_type())
    .collect();
  assert_eq!(
    roots,
    [EntryType::Level1 {
      supports_lowercase: false,
      supports_rrip: false
    }],
    "the rebuild should hold one plain directory tree and nothing else"
  );
}

#[test]
fn round_trips_the_native_identifiers_of_a_plain_image() {
  let source = image(vec![
    file("ZED.BIN", b"z"),
    directory(
      "DIR_A",
      vec![file(
        "INNER.BIN",
        b"inner",
      )],
    ),
    file("NOEXTENSION", b"plain"),
  ]);

  let rebuilt = Iso9660
    .encode(Vec::new(), &Iso9660.decode(&source, None).unwrap(), None)
    .unwrap();

  assert_eq!(
    identifiers(&rebuilt),
    identifiers(&source),
    "the identifiers the disc ends up carrying should be the ones it carried"
  );
}

#[test]
fn supplies_the_version_suffix_the_decoder_strips() {
  let source = image(vec![file(
    "GAME.BIN",
    b"x",
  )]);

  assert_eq!(
    identifiers(&source),
    ["GAME.BIN;1"],
    "a Level 1 file identifier carries a version"
  );

  let artifacts = Iso9660.decode(&source, None).unwrap();
  assert_eq!(
    names(&artifacts)[1..],
    ["GAME.BIN"],
    "the artifact should be named without the version"
  );

  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();
  assert_eq!(identifiers(&rebuilt), ["GAME.BIN;1"]);
}

#[test]
fn gives_a_directory_an_identifier_with_no_version_and_no_extension() {
  let source = image(vec![directory(
    "SUBDIR",
    vec![file("F.BIN", b"x")],
  )]);
  let rebuilt = Iso9660
    .encode(Vec::new(), &Iso9660.decode(&source, None).unwrap(), None)
    .unwrap();

  let image = IsoImage::open(Cursor::new(rebuilt)).unwrap();
  let root = plain_root(&image);

  let names: Vec<String> = root
    .iter(&image)
    .entries()
    .map(|entry| {
      let entry = entry.unwrap();
      String::from_utf8_lossy(entry.record.name()).into_owned()
    })
    .collect();

  assert!(
    names.contains(&"SUBDIR".to_owned()),
    "the directory should be named plainly: {names:?}"
  );
  assert!(
    !names.iter().any(|name| name.contains(';')),
    "a directory identifier takes no version: {names:?}"
  );
}

#[test]
fn reads_the_iso_9660_names_of_an_image_carrying_both_extensions() {
  // The shape `mkisofs -R -J` produces. Rock Ridge names these files
  // `lower.bin` and `noextension` and holds `LOWER.BIN;1` and `NOEXTENS.;1`
  // beside them, and what this codec reads has to be the latter: a rebuild for
  // an older reader is built from the names that reader will see.
  let Some(source) = mkisofs_both_extensions(
    "BOTH",
    &[
      ("lower.bin", b"lower"),
      ("noextension", b"plain"),
    ],
  ) else {
    eprintln!("skipping: mkisofs is not installed");
    return;
  };

  let artifacts = Iso9660.decode(&source, None).unwrap();

  assert_eq!(artifacts[0].name, METADATA_NAME);
  assert_eq!(
    metadata(&artifacts).volume_identifier,
    "BOTH",
    "the identifier comes from the primary descriptor, not the Joliet one"
  );
  assert_eq!(
    names(&artifacts)[1..],
    [
      "LOWER.BIN",
      "NOEXTENS."
    ],
    "these should be the ISO 9660 identifiers, not the Rock Ridge names"
  );
}

#[test]
fn accepts_every_name_level_1_does_hold() {
  for name in [
    "A",
    "A.B",
    "ABCDEFGH.123",
    "ABCDEFGH",
    "FILE.",
    "A/B/C.BIN",
    "DIR_1/FILE_2.BIN",
    "NUM12345.NUM",
  ] {
    assert!(
      encode_one(name).is_ok(),
      "'{name}' is a Level 1 identifier and should have been accepted"
    );
  }
}

#[test]
fn refuses_a_name_level_1_cannot_hold() {
  let mut accepted = Vec::new();

  for name in [
    "lowercase.bin",             // lower case
    "a-very-long-file-name.txt", // too long, and `-` is not in the charset
    "THISNAMEISWAYTOOLONG.BIN",  // 19 characters before the dot
    "LONGNAME.EXTENSION",        // 9 characters after it
    "WITH SPACE.BIN",            // a space
    "WITH;1.BIN",                // a `;` is how a version is spelled
    "FILE.BIN;1",                // a version left in an artifact name
    "UPPER.BI.N",                // a second dot, which is not a Level 1 extension
    "SUB.DIR/FILE.BIN",          // a directory name carrying an extension
    "lowerdir/FILE.BIN",         // a directory name in lower case
    ".BIN",                      // no name at all
    "/FILE.BIN",                 // an empty first component
    "A//FILE.BIN",               // an empty component between two others
    "",                          // nothing at all
  ] {
    if encode_one(name).is_ok() {
      accepted.push(name);
    }
  }

  assert!(
    accepted.is_empty(),
    "these are not Level 1 identifiers and should have been refused: {accepted:?}"
  );
}

#[test]
fn names_the_file_it_refuses() {
  for name in [
    "lowercase.bin",
    "A//FILE.BIN",
    "SUB.DIR/FILE.BIN",
  ] {
    let err = encode_one(name).unwrap_err().to_string();

    assert!(err.contains("Level 1"), "{err}");
    assert!(err.contains(name), "the error should name the file: {err}");
  }
}

#[test]
fn decodes_an_image_no_reader_was_involved_in_writing() {
  let Some(source) = mkisofs(
    "PROBE",
    &[
      ("ZED.BIN", b"z"),
      ("ALPHA.BIN", b"a"),
      ("DIR_A/INNER.BIN", b"inner"),
      ("NOEXTENSION", b"plain"),
    ],
    &[],
  ) else {
    eprintln!("skipping: mkisofs is not installed");
    return;
  };

  let artifacts = Iso9660.decode(&source, None).unwrap();

  assert_eq!(artifacts[0].name, METADATA_NAME);
  assert_eq!(metadata(&artifacts).volume_identifier, "PROBE");
  assert_eq!(
    names(&artifacts)[1..],
    [
      "ALPHA.BIN",
      "NOEXTENS.",
      "ZED.BIN",
      "DIR_A/INNER.BIN"
    ],
    "cdrtools pads an extensionless name to `NOEXTENS.;1`, and this codec \
     should hand that identifier on as it stands"
  );

  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  assert_eq!(
    read_back(&rebuilt),
    read_back(&source),
    "a rebuild should hold the files the source held"
  );
  assert_eq!(
    identifiers(&rebuilt),
    identifiers(&source),
    "and carry the same identifiers, even though two writers made them"
  );
}

#[test]
fn reads_back_what_it_writes_through_a_reader_that_is_not_hadris() {
  let rebuilt = Iso9660
    .encode(Vec::new(), &Iso9660.decode(&sample(), None).unwrap(), None)
    .unwrap();

  // libarchive reads ISO 9660 in its own right, and drops the `;1` from what it
  // lists, so this is a reader that owes the codec nothing.
  let path = std::env::temp_dir().join(format!(
    "ludared-iso9660-readback-{}.iso",
    std::process::id()
  ));
  std::fs::write(&path, &rebuilt).unwrap();

  let listed = Command::new("bsdtar").arg("-tf").arg(&path).output();
  std::fs::remove_file(&path).ok();

  let Ok(listed) = listed else {
    eprintln!("skipping: bsdtar is not installed");
    return;
  };

  let listing = String::from_utf8_lossy(&listed.stdout).into_owned();

  for path in [
    "Z.BIN",
    "M.BIN",
    "A/FIRST.BIN",
    "DIR/INNER.BIN",
  ] {
    assert!(
      listing.lines().any(|line| line == path),
      "`{path}` should be listed by libarchive: {listing}"
    );
  }
}
