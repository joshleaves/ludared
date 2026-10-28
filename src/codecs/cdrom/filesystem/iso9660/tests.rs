use hadris_iso::file::EntryType;
use hadris_iso::read::IsoImage;
use hadris_iso::read::PathSeparator;
use hadris_iso::write::File as IsoFile;
use hadris_iso::write::InputFiles;
use hadris_iso::write::IsoImageWriter;
use hadris_iso::write::options::CreationFeatures;
use hadris_iso::write::options::IsoFormatOptions;
use std::io::Cursor;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use super::metadata::BLOCK_SIZE;
use super::metadata::IsoMetadata;
use super::metadata::METADATA_NAME;
use super::volume::DESCRIPTOR_SECTOR;
use super::volume::SYSTEM_AREA_SIZE;
use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::DecodedArtifact;
use crate::codecs::cdrom::filesystem::iso9660::Iso9660;

/// The first byte of the image's descriptor set.
const DESCRIPTOR_START: usize = DESCRIPTOR_SECTOR as usize * BLOCK_SIZE;

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

/// Replaces the contents of one file, leaving every other artifact alone.
fn replacing(artifacts: &[DecodedArtifact], name: &str, data: Vec<u8>) -> Vec<DecodedArtifact> {
  let mut artifacts = copy(artifacts);

  let file = artifacts
    .iter_mut()
    .find(|artifact| artifact.name == name)
    .unwrap_or_else(|| panic!("the decode should hold a file named '{name}'"));

  file.data = data;

  artifacts
}

/// Replaces the metadata of a decode, leaving every file artifact alone.
fn with_metadata(artifacts: &[DecodedArtifact], metadata: &IsoMetadata) -> Vec<DecodedArtifact> {
  let mut artifacts = copy(artifacts);
  artifacts[0].data = metadata.to_json().unwrap();

  artifacts
}

/// Decodes an image and rebuilds it, which for an unchanged decode has to give
/// the image back byte for byte.
fn rebuild(image: &[u8]) -> Vec<u8> {
  let artifacts = Iso9660.decode(image, None).unwrap();
  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  assert_eq!(
    rebuilt.len(),
    image.len(),
    "the rebuild should be as long as the image it came from"
  );

  assert_same(
    &rebuilt,
    image,
    "an unchanged decode and rebuild should reproduce the image byte for byte",
  );

  rebuilt
}

/// Asserts that two images hold the same bytes, naming the first one that does
/// not, which says far more than two arrays printed side by side.
fn assert_same(rebuilt: &[u8], original: &[u8], what: &str) {
  if rebuilt == original {
    return;
  }

  assert_eq!(rebuilt.len(), original.len(), "{what}: the lengths differ");

  let offset = rebuilt
    .iter()
    .zip(original)
    .position(|(a, b)| a != b)
    .expect("two unequal byte slices should differ somewhere");

  panic!(
    "{what}: the first difference is at byte {offset:#X}, block {} + {:#X}: rebuilt {:#04X}, \
     original {:#04X}",
    offset / BLOCK_SIZE,
    offset % BLOCK_SIZE,
    rebuilt[offset],
    original[offset]
  );
}

/// Rebuilds the artifacts as they stand, without requiring the image's bytes.
fn encode(artifacts: &[DecodedArtifact]) -> Vec<u8> {
  Iso9660.encode(Vec::new(), artifacts, None).unwrap()
}

fn build(volume_identifier: &str, files: Vec<IsoFile>) -> Vec<u8> {
  let options = IsoFormatOptions {
    volume_name: volume_identifier.to_owned(),
    system_id: None,
    volume_set_id: None,
    publisher_id: None,
    preparer_id: None,
    application_id: None,
    sector_size: BLOCK_SIZE,
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

/// An ISO built the way Hadris builds one, which is how the fixtures these
/// tests use most were made.
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
/// path, the contents, and the block it sits at.
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

/// The files an image's directory tree holds: the path of each, the block it
/// starts at, and how long it is.
///
/// Walked through the image's own records rather than through a library, so
/// that a test comparing where a rebuild put things does not take the word of
/// the code that put them there.
fn tree(image: &[u8]) -> Vec<(String, u32, u32)> {
  let mut found = Vec::new();

  walk_records(image, |path, _, extent, length, is_directory, _| {
    if !is_directory {
      found.push((path.to_owned(), extent, length));
    }
  });

  found.sort();
  found
}

/// Walks an image's own directory records, handing each to `visit` as its path
/// with the version stripped, the byte offset its record sits at, the block it
/// starts at, its length, whether it describes a directory, and the identifier
/// it carries.
///
/// Walked through the records rather than through a library, so that a test
/// comparing where a rebuild put things does not take the word of the code that
/// put them there.
fn walk_records(image: &[u8], mut visit: impl FnMut(&str, usize, u32, u32, bool, &str)) {
  let mut pending = vec![(both_ended(image, DESCRIPTOR_START + 156 + 2), String::new())];
  let mut seen = std::collections::BTreeSet::new();

  while let Some((block, path)) = pending.pop() {
    assert!(seen.insert(block), "the tree should not loop");

    let start = block as usize * BLOCK_SIZE;

    // A directory's records run for the length its own record declares, which a
    // directory of more entries than fit one block spreads over several. Its own
    // record leads the extent, so the length sits ten bytes into the block.
    let length = both_ended(image, start + 10) as usize;
    let mut offset = 0;

    while offset + 34 <= length {
      let size = image[start + offset] as usize;

      // A record that does not fit in the room left in its block starts the
      // next one, and the bytes it left behind are a pad of zeroes.
      if size < 34 {
        offset = (offset.div_euclid(BLOCK_SIZE) + 1) * BLOCK_SIZE;
        continue;
      }

      let at = start + offset;
      let record = &image[at..at + size];
      let name_len = record[32] as usize;
      let identifier = String::from_utf8_lossy(&record[33..33 + name_len]).into_owned();
      let leaf = identifier.split(';').next().unwrap_or_default().to_owned();

      let child = if path.is_empty() {
        leaf
      } else {
        format!("{path}/{leaf}")
      };

      let is_directory = record[25] & 0x02 != 0;
      let extent = both_ended(image, at + 2);
      let data_len = both_ended(image, at + 10);

      if is_directory && identifier.as_bytes() != b"\x00" && identifier.as_bytes() != b"\x01" {
        pending.push((extent, child.clone()));
      }

      visit(&child, at, extent, data_len, is_directory, &identifier);
      offset += size;
    }
  }
}

/// Where each file the image holds sits, by path.
fn blocks(bytes: &[u8]) -> Vec<(String, u32)> {
  tree(bytes)
    .into_iter()
    .map(|(path, extent, _)| (path, extent))
    .collect()
}

/// The contents of one file, read out of the image through its own records.
fn contents_at(image: &[u8], path: &str) -> Vec<u8> {
  let (_, extent, length) = tree(image)
    .into_iter()
    .find(|(held, _, _)| held == path)
    .unwrap_or_else(|| panic!("the image should hold '{path}'"));

  let start = extent as usize * BLOCK_SIZE;

  image[start..start + length as usize].to_vec()
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
/// codec deliberately leaves alone. `None` where cdrtools is not installed, so
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

/// Fills the System Area of an image with bytes that are none of them zero.
///
/// ISO 9660 reserves the blocks leading the descriptor set and says nothing of
/// what they hold, so a platform is free to keep its own boot data there: the
/// signature a console looks for before it will boot a disc is one of those
/// bytes. Every byte differs from zero and from the one after it, so a rebuild
/// that zeroed the area, or kept only what it happened to recognise, shows up.
fn stamp_system_area(image: &mut [u8]) {
  let system_area = &mut image[..SYSTEM_AREA_SIZE];

  assert!(
    system_area.iter().all(|byte| *byte == 0),
    "an image Hadris wrote should carry an empty system area"
  );

  for (index, byte) in system_area.iter_mut().enumerate() {
    *byte = (index % 251) as u8 + 1;
  }
}

/// The message a decode an image refused came back with.
///
/// `DecodedArtifact` carries no `Debug`, so a test cannot simply unwrap the
/// error side of a decode.
fn decode_error(image: &[u8], args: Option<&str>) -> String {
  match Iso9660.decode(image, args) {
    Ok(artifacts) => panic!(
      "the image should have been refused, and {} artifacts came back",
      artifacts.len()
    ),
    Err(err) => err.to_string(),
  }
}

/// Reads a both-ended number the way ISO 9660 writes one.
fn both_ended(image: &[u8], offset: usize) -> u32 {
  u32::from_le_bytes(image[offset..offset + 4].try_into().unwrap())
}

/// The byte offset of the record a file's identifier sits in, found by walking
/// the image's own directories.
///
/// The tests that need it reach for bytes no reader would hand out: a record
/// pointing at blocks the image does not hold, or one flagged as something this
/// codec refuses.
fn record_offset(image: &[u8], identifier: &str) -> usize {
  let mut found = None;

  walk_records(image, |_, offset, _, _, _, name| {
    if found.is_none() && name == identifier {
      found = Some(offset);
    }
  });

  found.unwrap_or_else(|| panic!("the image should hold a record named '{identifier}'"))
}

/// Declares a Volume Space Size in an image's primary descriptor, the way a
/// mastering tool that wrote a larger volume would have.
fn declare_volume_space_size(image: &mut [u8], blocks: u32) {
  let at = DESCRIPTOR_START + 80;

  image[at..at + 4].copy_from_slice(&blocks.to_le_bytes());
  image[at + 4..at + 8].copy_from_slice(&blocks.to_be_bytes());
}

/// Points a file's record at blocks the image does not hold, the way a
/// mixed-mode disc describes a file kept on the track that follows.
fn move_beyond_the_image(image: &mut [u8], identifier: &str, block: u32, length: u32) {
  let offset = record_offset(image, identifier);

  image[offset + 2..offset + 6].copy_from_slice(&block.to_le_bytes());
  image[offset + 6..offset + 10].copy_from_slice(&block.to_be_bytes());
  image[offset + 10..offset + 14].copy_from_slice(&length.to_le_bytes());
  image[offset + 14..offset + 18].copy_from_slice(&length.to_be_bytes());
}

/// Flags a record as something this codec refuses to rebuild.
fn flag_record(image: &mut [u8], identifier: &str, flags: u8) {
  let offset = record_offset(image, identifier);

  image[offset + 25] |= flags;
}

// ---------------------------------------------------------------- the codec

#[test]
fn recognizes_an_iso_9660_image() {
  assert_eq!(
    Iso9660.can_handle(&sample()).unwrap(),
    CodecHandlingConfidence::Certain
  );
}

#[test]
fn refuses_input_that_is_not_an_iso_9660_image() {
  for data in [
    vec![],
    vec![0u8; 16 * BLOCK_SIZE],
    {
      let mut bytes = sample();
      bytes[16 * BLOCK_SIZE] = 0x02; // a supplementary, not a primary, descriptor
      bytes
    },
    {
      let mut bytes = sample();
      bytes[16 * BLOCK_SIZE + 1] = b'X'; // `CD001` no longer identifies itself
      bytes
    },
  ] {
    assert_eq!(
      Iso9660.can_handle(&data).unwrap(),
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

    assert_eq!(
      metadata(&Iso9660.decode(&rebuild(&source), None).unwrap()).volume_identifier,
      volume,
      "and the rebuild"
    );
  }
}

#[test]
fn keeps_a_volume_identifier_the_charset_would_rather_not_write() {
  // A writer that folded this into `MY_SATURN_DISC` would change the image, so
  // the exact value is what the round trip has to keep.
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
}

#[test]
fn writes_the_metadata_as_readable_json() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();
  let json = String::from_utf8(artifacts[0].data.clone()).unwrap();
  let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

  assert_eq!(parsed["volume_identifier"], "LUDARED");
  assert_eq!(
    parsed["file_order"],
    serde_json::json!([
      "Z.BIN",
      "M.BIN",
      "A/FIRST.BIN",
      "DIR/INNER.BIN"
    ])
  );

  let layout = &parsed["layout"];

  assert_eq!(layout["image_sectors"], 27);
  assert_eq!(layout["volume_space_size"], 27);
  // A field holding the value ISO 9660 mandates is left out, so that the
  // metadata says what is unusual about the image and nothing more.
  assert!(
    layout["primary"].get("logical_block_size").is_none(),
    "the block size the codec works in needs no recording"
  );
  assert_eq!(
    metadata(&artifacts).layout.primary.logical_block_size,
    2048,
    "and reading it back fills the mandated value in"
  );
  assert_eq!(layout["path_table"]["entries"][0]["name"], "\u{0}");
  assert!(layout["system_area"].is_null(), "an empty area is left out");

  // The terminator that closes the descriptor set is not a structure the codec
  // models, so it is preserved as the bytes it is.
  let opaque = layout["opaque"]
    .as_array()
    .expect("the terminator should be preserved");

  assert_eq!(opaque.len(), 1);
  assert_eq!(opaque[0]["offset"], 34816, "the block it sits in");
  assert!(
    opaque[0]["data"]
      .as_str()
      .unwrap()
      .starts_with("ff4344303031"),
    "the descriptor set terminator"
  );

  // A directory is its records, the entries for itself and its parent first.
  let root = &layout["directories"][0];
  assert_eq!(root["path"], "");
  assert_eq!(root["records"][0]["name"], "\u{0}");
  assert_eq!(root["records"][1]["name"], "\u{1}");
  assert_eq!(root["records"].as_array().unwrap().len(), 6);

  assert!(
    json.contains('\n'),
    "the metadata should be readable as it stands"
  );
}

#[test]
fn carries_the_system_area_through_a_rebuild() {
  let mut source = sample();
  stamp_system_area(&mut source);

  let rebuilt = rebuild(&source);

  assert!(
    rebuilt
      .get(..SYSTEM_AREA_SIZE)
      .is_some_and(|area| area == &source[..SYSTEM_AREA_SIZE]),
    "the system area should come back byte for byte"
  );
}

#[test]
fn rebuilds_naming_the_volume_the_metadata_records() {
  // One build, compared with itself: the fixture builder stamps the time it ran
  // into the descriptor and the root's record, so two builds of the same fixture
  // are not necessarily the same image.
  let mut expected = sample();
  let artifacts = Iso9660.decode(&expected, None).unwrap();

  let mut renamed = metadata(&artifacts);
  renamed.volume_identifier = "RENAMED_VOLUME".to_owned();

  let rebuilt = encode(&with_metadata(&artifacts, &renamed));
  let again = Iso9660.decode(&rebuilt, None).unwrap();

  assert_eq!(metadata(&again).volume_identifier, "RENAMED_VOLUME");

  // The rest of the image is untouched: only the identifier changed.
  expected[DESCRIPTOR_START + 40..DESCRIPTOR_START + 72].copy_from_slice(&{
    let mut field = [b' '; 32];
    field[..14].copy_from_slice(b"RENAMED_VOLUME");
    field
  });

  assert_same(
    &rebuilt,
    &expected,
    "only the volume identifier should differ from the image the metadata came from",
  );
}

// ------------------------------------------------------------ changing files

#[test]
fn rebuilds_after_a_file_changes_size() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  for (name, data) in [
    ("Z.BIN", vec![b'z'; 9000]),
    ("A/FIRST.BIN", b"first".to_vec()),
    ("M.BIN", b"m".to_vec()),
  ] {
    let rebuilt = encode(&replacing(&artifacts, name, data.clone()));
    let contents = read_back(&rebuilt);

    assert_eq!(contents.len(), 4, "the rebuild should hold the same files");
    assert_eq!(
      contents
        .iter()
        .find(|(path, _)| path == name)
        .map(|(_, held)| held.clone()),
      Some(data),
      "'{name}' should hold what it was given"
    );
  }
}

#[test]
fn keeps_a_file_of_the_same_size_where_it_was() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let rebuilt = encode(&replacing(&artifacts, "M.BIN", b"M".to_vec()));

  assert_eq!(
    blocks(&rebuilt),
    blocks(&source),
    "a replacement of the same size should move nothing"
  );
  assert_eq!(read_back(&rebuilt)[2], ("M.BIN".to_owned(), b"M".to_vec()));
}

/// A file that shrinks keeps the blocks it held: the room it no longer fills is
/// left as it is, so nothing else has to move either.
#[test]
fn keeps_a_smaller_file_where_it_was() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let rebuilt = encode(&replacing(&artifacts, "Z.BIN", b"zz".to_vec()));

  assert_eq!(blocks(&rebuilt), blocks(&source));
  assert_eq!(read_back(&rebuilt)[3], ("Z.BIN".to_owned(), b"zz".to_vec()));
}

/// A file that grows into the room its own blocks already give it stays where
/// it was too.
#[test]
fn grows_a_file_within_its_own_blocks() {
  let source = image(vec![file(
    "BIG.BIN",
    &[b'z'; 3000],
  )]);
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let rebuilt = encode(&replacing(&artifacts, "BIG.BIN", vec![b'z'; 4096]));

  assert_eq!(blocks(&rebuilt), blocks(&source));
  assert_eq!(
    read_back(&rebuilt),
    [("BIG.BIN".to_owned(), vec![b'z'; 4096])]
  );
}

/// A file that no longer fits the blocks it held is given room: the first free
/// room in the image, and room after the image's end when it has none.
#[test]
fn moves_a_file_whose_own_blocks_cannot_hold_it() {
  let source = image(vec![
    file("FIRST.BIN", &[b'a'; 4096]),
    file("SECOND.BIN", b"second"),
  ]);
  let artifacts = Iso9660.decode(&source, None).unwrap();
  let before = blocks(&source)
    .into_iter()
    .find(|(path, _)| path == "FIRST.BIN")
    .map(|(_, block)| block)
    .unwrap();

  let rebuilt = encode(&replacing(&artifacts, "FIRST.BIN", vec![b'a'; 8192]));
  let after = blocks(&rebuilt)
    .into_iter()
    .find(|(path, _)| path == "FIRST.BIN")
    .map(|(_, block)| block)
    .unwrap();

  assert_ne!(after, before, "the file should have moved to find room");
  assert_eq!(contents_at(&rebuilt, "FIRST.BIN"), vec![b'a'; 8192]);
  assert_eq!(contents_at(&rebuilt, "SECOND.BIN"), b"second".to_vec());

  // Room was found by growing the image, so the blocks the file took are in it.
  assert!(after as usize + 4 <= rebuilt.len() / BLOCK_SIZE);
}

/// A standalone image is one that ends where its last block does: the volume it
/// declares ends with it, and a file that outgrows the room it has makes the
/// image longer. The blocks it gains are the volume's, and every file that fits where
/// it was stays where it was.
#[test]
fn grows_a_standalone_image_past_the_blocks_it_held() {
  let source = image(vec![
    file("FIRST.BIN", &[b'a'; 4096]),
    file("SECOND.BIN", &[b'b'; 4096]),
  ]);
  let artifacts = Iso9660.decode(&source, None).unwrap();

  assert_eq!(
    both_ended(&source, DESCRIPTOR_START + 80) as usize * BLOCK_SIZE,
    source.len(),
    "the fixture should be an image whose volume ends with it"
  );

  let before = blocks(&source)
    .into_iter()
    .find(|(path, _)| path == "SECOND.BIN")
    .map(|(_, block)| block)
    .unwrap();

  let rebuilt = encode(&replacing(&artifacts, "FIRST.BIN", vec![b'a'; 8192]));

  assert!(
    rebuilt.len() > source.len(),
    "the image should have grown past the blocks it held"
  );
  assert_eq!(
    both_ended(&rebuilt, DESCRIPTOR_START + 80) as usize * BLOCK_SIZE,
    rebuilt.len(),
    "the volume should cover the image it belongs to"
  );
  assert_eq!(
    contents_at(&rebuilt, "FIRST.BIN"),
    vec![b'a'; 8192],
    "the file should hold what it was given"
  );

  let after = blocks(&rebuilt)
    .into_iter()
    .find(|(path, _)| path == "FIRST.BIN")
    .map(|(_, block)| block)
    .unwrap();

  assert!(
    after as usize + 4 <= rebuilt.len() / BLOCK_SIZE,
    "the grown image should hold the blocks the file takes"
  );
  assert_eq!(
    blocks(&rebuilt)
      .into_iter()
      .find(|(path, _)| path == "SECOND.BIN")
      .map(|(_, block)| block),
    Some(before),
    "the file that fits where it was should not have moved"
  );
  assert_eq!(
    read_back(&rebuilt),
    [
      ("FIRST.BIN".to_owned(), vec![b'a'; 8192]),
      ("SECOND.BIN".to_owned(), vec![b'b'; 4096]),
    ],
    "every file should read back out of the grown image"
  );
}

#[test]
fn rebuilds_after_a_file_shrinks_to_nothing() {
  let source = image(vec![
    file("KEEP.BIN", b"keep"),
    file("DROP.BIN", b"drop"),
  ]);
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let rebuilt = encode(&replacing(&artifacts, "DROP.BIN", Vec::new()));

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
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let rebuilt = encode(&replacing(
    &artifacts,
    "EMPTY.BIN",
    b"now it has something in it".to_vec(),
  ));

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

  let mut extended = with_metadata(&artifacts, &meta);
  extended.push(artifact("EXTRA.BIN", b"extra"));

  let rebuilt = encode(&extended);
  let contents = read_back(&rebuilt);
  let paths: Vec<&str> = contents.iter().map(|(path, _)| path.as_str()).collect();

  assert!(
    paths.contains(&"EXTRA.BIN"),
    "the added file should be in the image: {paths:?}"
  );
  assert_eq!(contents.len(), read_back(&source).len() + 1);
  assert_eq!(
    contents
      .iter()
      .find(|(path, _)| path == "EXTRA.BIN")
      .map(|(_, data)| data.clone()),
    Some(b"extra".to_vec())
  );
}

#[test]
fn adds_a_file_below_a_directory() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let mut meta = metadata(&artifacts);
  meta.file_order.push("A/ADDED.BIN".to_owned());

  let mut extended = with_metadata(&artifacts, &meta);
  extended.push(artifact("A/ADDED.BIN", b"added"));

  let rebuilt = encode(&extended);

  assert_eq!(
    read_back(&rebuilt)
      .into_iter()
      .find(|(path, _)| path == "A/ADDED.BIN"),
    Some(("A/ADDED.BIN".to_owned(), b"added".to_vec()))
  );

  // The directories the image held still describe the same files.
  assert_eq!(read_back(&source).len() + 1, read_back(&rebuilt).len());
}

/// The metadata, the layout and the artifacts have to agree exactly, and each
/// way they can disagree is refused with what disagrees named: a file the
/// metadata does not name, one it names that is not an artifact, one it names
/// twice, and one the layout holds that no artifact carries.
#[test]
fn refuses_an_artifact_set_the_layout_disagrees_about() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  let mut unplaced = copy(&artifacts);
  unplaced.push(artifact("EXTRA.BIN", b"extra"));

  let mut missing = metadata(&artifacts);
  missing.file_order.push("MISSING.BIN".to_owned());

  let mut twice = metadata(&artifacts);
  twice.file_order.push("Z.BIN".to_owned());

  let mut uncarried = metadata(&artifacts);
  uncarried.file_order.retain(|path| path != "M.BIN");

  let mut dropped = with_metadata(&artifacts, &uncarried);
  dropped.retain(|held| held.name != "M.BIN");

  for (what, refused, named) in [
    (
      "an artifact the metadata does not name",
      unplaced,
      "EXTRA.BIN",
    ),
    (
      "a name that is not an artifact",
      with_metadata(&artifacts, &missing),
      "MISSING.BIN",
    ),
    (
      "a name the metadata repeats",
      with_metadata(&artifacts, &twice),
      "more than once",
    ),
    // A file whose artifact is gone is not a file the image no longer holds:
    // only an edit to the layout says that, and a rebuild will not guess at it.
    ("a file no artifact carries", dropped, "M.BIN"),
  ] {
    let err = Iso9660
      .encode(Vec::new(), &refused, None)
      .unwrap_err()
      .to_string();

    assert!(
      err.contains(named),
      "{what} should be refused for '{named}': {err}"
    );
  }
}

#[test]
fn refuses_a_file_added_below_a_directory_the_image_does_not_hold() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  let mut meta = metadata(&artifacts);
  meta.file_order.push("NOWHERE/ADDED.BIN".to_owned());

  let mut extended = with_metadata(&artifacts, &meta);
  extended.push(artifact("NOWHERE/ADDED.BIN", b"added"));

  let err = Iso9660.encode(Vec::new(), &extended, None).unwrap_err();

  assert!(
    err.to_string().contains("NOWHERE"),
    "the directory should be named: {err}"
  );
}

#[test]
fn refuses_a_name_level_1_cannot_hold() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();
  let mut accepted = Vec::new();

  for name in [
    "lowercase.bin",             // lower case
    "a-very-long-file-name.txt", // too long, and `-` is not in the charset
    "THISNAMEISWAYTOOLONG.BIN",  // 19 characters before the dot
    "LONGNAME.EXTENSION",        // 9 characters after it
    "WITH SPACE.BIN",            // a space
    "WITH;1.BIN",                // a `;` is how a version is spelled
    "UPPER.BI.N",                // a second dot, which is not a Level 1 extension
    ".BIN",                      // no name at all
    "",                          // nothing at all
    "..",                        // a name that walks up a directory rather than one
  ] {
    let mut meta = metadata(&artifacts);
    meta.file_order.push(name.to_owned());

    let mut extended = with_metadata(&artifacts, &meta);
    extended.push(artifact(name, b"x"));

    if let Err(err) = Iso9660.encode(Vec::new(), &extended, None) {
      let err = err.to_string();

      assert!(err.contains("Level 1"), "{err}");
      assert!(err.contains(name), "the error should name the file: {err}");
    } else {
      accepted.push(name);
    }
  }

  assert!(
    accepted.is_empty(),
    "these are not Level 1 identifiers and should have been refused: {accepted:?}"
  );
}

#[test]
fn accepts_every_name_level_1_does_hold() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  for name in [
    "AB",
    "A.B",
    "ABCDEFGH.123",
    "ABCDEFGH",
    "FILE.",
    "A/ADDED.B",
    "A/FILE_2.B",
    "NUM12345.NUM",
  ] {
    let mut meta = metadata(&artifacts);
    meta.file_order.push(name.to_owned());

    let mut extended = with_metadata(&artifacts, &meta);
    extended.push(artifact(name, b"x"));

    let rebuilt = Iso9660.encode(Vec::new(), &extended, None).unwrap();

    assert!(
      read_back(&rebuilt).iter().any(|(path, _)| path == name),
      "'{name}' is a Level 1 identifier and should have been written"
    );
  }
}

// --------------------------------------------------------- the layout itself

#[test]
fn never_puts_the_metadata_inside_the_image() {
  let rebuilt = rebuild(&sample());

  let paths: Vec<String> = read_back(&rebuilt)
    .into_iter()
    .map(|(path, _)| path)
    .collect();

  assert!(
    !paths.iter().any(|path| path.ends_with(".json")),
    "the metadata was laid into the image: {paths:?}"
  );
}

#[test]
fn takes_the_metadata_off_the_front_whatever_it_is_named() {
  let mut renamed = copy(&Iso9660.decode(&sample(), None).unwrap());
  renamed[0].name = "ANYTHING_AT_ALL".to_owned();

  let rebuilt = encode(&renamed);

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
      &Iso9660.decode(&sample(), None).unwrap()[0].data,
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
    br#"{"volume_identifier":"X","file_order":[["A.BIN"]]}"#,
    br#"{"volume_identifier":"X","file_order":[],"layout":{}}"#,
    br#"{"volume_identifier":42,"file_order":[],"layout":null}"#,
    br#"{"volume_identifier":"X","file_order":[],"layout":{"image_sectors":1,"volume_space_size":1,"primary":{},"path_table":{},"directories":[],"system_area":"zz"}}"#,
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

  // The round trip has to hold both DATA.BIN records where they were, which is
  // what the byte-for-byte rebuild asserts.
  rebuild(&source);
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

  let rebuilt = rebuild(&source);

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

  rebuild(&source);
}

#[test]
fn refuses_input_holding_no_volume_descriptor() {
  assert!(Iso9660.decode(&vec![0u8; 16 * BLOCK_SIZE], None).is_err());
  assert!(Iso9660.decode(&[], None).is_err());
}

/// An image that does not hold whole blocks, or ends before the structures it
/// describes, is refused rather than read as far as it goes.
#[test]
fn refuses_a_truncated_image() {
  let source = sample();

  let err = decode_error(&source[..source.len() - 1], None);
  assert!(err.contains("whole number of"), "{err}");

  // Nothing but the System Area, so no descriptor set at all.
  assert!(Iso9660.decode(&source[..DESCRIPTOR_START], None).is_err());

  // Cut before the path tables the descriptor names.
  let err = decode_error(&source[..18 * BLOCK_SIZE], None);
  assert!(
    err.contains("path table") || err.contains("logical block"),
    "{err}"
  );
}

/// A descriptor set that goes on without a terminator is malformed, however
/// valid each descriptor in it looks: the decode stops counting at 64 rather
/// than walking past the structures the set leads.
#[test]
fn refuses_a_descriptor_set_that_never_ends() {
  let mut source = sample();
  source.resize(100 * BLOCK_SIZE, 0);

  let primary = source[DESCRIPTOR_START..DESCRIPTOR_START + BLOCK_SIZE].to_vec();

  for sector in 17..=80 {
    let at = sector as usize * BLOCK_SIZE;
    source[at..at + BLOCK_SIZE].copy_from_slice(&primary);
  }

  let err = decode_error(&source, None);

  assert!(
    err.contains("not terminated after 64"),
    "the set should have been refused at its limit: {err}"
  );
}

/// A directory whose record leads back to a directory the walk has already
/// opened: the tree loops, and the decode says so rather than walking it
/// forever.
#[test]
fn refuses_a_tree_whose_directories_point_at_each_other_in_a_loop() {
  let mut source = sample();
  let root = both_ended(&source, DESCRIPTOR_START + 156 + 2);

  // The record for 'DIR' now describes the root's own blocks.
  let offset = record_offset(&source, "DIR");
  source[offset + 2..offset + 6].copy_from_slice(&root.to_le_bytes());
  source[offset + 6..offset + 10].copy_from_slice(&root.to_be_bytes());

  let err = decode_error(&source, None);

  assert!(
    err.contains("do not form a tree"),
    "the loop should have been reported: {err}"
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
    directory("SUBDIR", vec![file("F.BIN", b"x")]),
  ]);

  let rebuilt = rebuild(&source);

  assert_eq!(
    identifiers(&rebuilt),
    identifiers(&source),
    "the identifiers the disc ends up carrying should be the ones it carried"
  );

  // A directory identifier carries neither a version nor an extension.
  let image = IsoImage::open(Cursor::new(rebuilt)).unwrap();
  let root = plain_root(&image);

  let directories: Vec<String> = root
    .iter(&image)
    .entries()
    .filter_map(|entry| {
      let entry = entry.unwrap();

      entry
        .is_directory()
        .then(|| String::from_utf8_lossy(entry.record.name()).into_owned())
    })
    .collect();

  assert!(
    directories.contains(&"SUBDIR".to_owned()),
    "the directory should be named plainly: {directories:?}"
  );
  assert!(
    !directories.iter().any(|name| name.contains(';')),
    "a directory identifier takes no version: {directories:?}"
  );
}

#[test]
fn round_trips_a_directory_whose_recorded_length_is_not_block_aligned() {
  let mut source = image(vec![file(
    "SHORT.BIN",
    b"x",
  )]);
  let root = both_ended(&source, DESCRIPTOR_START + 156 + 2) as usize;
  let start = root * BLOCK_SIZE;
  let mut length = 0usize;

  while source[start + length] != 0 {
    length += source[start + length] as usize;
  }

  assert!(length > 0 && length < BLOCK_SIZE);
  set_both_ended(&mut source, DESCRIPTOR_START + 156 + 10, length as u32);
  set_both_ended(&mut source, start + 10, length as u32);

  rebuild(&source);
}

fn set_both_ended(image: &mut [u8], offset: usize, value: u32) {
  image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
  image[offset + 4..offset + 8].copy_from_slice(&value.to_be_bytes());
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

  let rebuilt = rebuild(&source);
  assert_eq!(identifiers(&rebuilt), ["GAME.BIN;1"]);
}

/// The image's own bytes outside its filesystem are not padding to be dropped:
/// a rebuild that lost them would hand back a disc that no longer holds what
/// its game reads.
#[test]
fn keeps_the_bytes_that_follow_the_filesystem() {
  let mut source = sample();

  // Data a mastering tool left after the filesystem, which nothing in the
  // directory tree points at. Filled out to whole blocks, as a track is.
  let mut trailing = b"a second track's worth of data, after the filesystem".to_vec();
  trailing.resize(trailing.len().next_multiple_of(BLOCK_SIZE), 0x5A);

  source.extend_from_slice(&trailing);

  let artifacts = Iso9660.decode(&source, None).unwrap();
  let layout = metadata(&artifacts).layout;

  assert_eq!(
    layout.image_sectors as usize * BLOCK_SIZE,
    source.len(),
    "the layout should record how long the image is"
  );
  assert!(
    !layout.opaque.is_empty(),
    "the bytes outside the filesystem should be recorded"
  );

  // And the rebuild writes them back where they lay, which the byte-for-byte
  // rebuild asserts.
  rebuild(&source);
}

#[test]
fn records_the_bytes_a_writer_left_in_the_descriptor() {
  // A mastering tool is free to keep its own data in the Application Use area of
  // the primary descriptor: ISO 9660 reserves those bytes and says nothing of
  // what they hold.
  let mut source = sample();
  let stamp = b"cdrtools 3.02a06, which ISO 9660 does not describe";

  source[DESCRIPTOR_START + 883..DESCRIPTOR_START + 883 + stamp.len()].copy_from_slice(stamp);

  let artifacts = Iso9660.decode(&source, None).unwrap();

  assert_eq!(
    metadata(&artifacts)
      .layout
      .primary
      .application_use
      .as_bytes(),
    stamp,
    "what the writer left there should be recorded as it stands"
  );

  rebuild(&source);
}

/// An image whose root directory holds its records in two blocks: the records
/// of the first end four bytes short of its boundary, and the record that would
/// have straddled it sits in the next block behind a zero byte — what a writer
/// that follows ISO 9660 leaves, since a record may not cross the boundary.
fn two_block_root() -> Vec<u8> {
  let mut files = vec![file("A.ODD", b"x")];

  files.extend((0..60).map(|index| file(&format!("F{index:03}.BIN"), b"x")));

  image(files)
}

/// The zero byte before the record the writer moved on is the record's place:
/// a rebuild has to write the record back in the block it sat in, marker and
/// all, which is what keeps the image byte for byte.
#[test]
fn round_trips_a_directory_whose_record_was_moved_on_to_the_next_block() {
  let source = two_block_root();

  let start = both_ended(&source, DESCRIPTOR_START + 156 + 2) as usize * BLOCK_SIZE;

  assert_eq!(
    source[start + 2044],
    0,
    "the first block should end in the marker the writer left"
  );
  assert_eq!(
    source[start + 2048],
    44,
    "the record that would have straddled the boundary should sit in the next block"
  );

  let rebuilt = rebuild(&source);

  assert_eq!(
    rebuilt[start + 2044],
    0,
    "the marker should come back where it was"
  );
  assert_directories_agree(&rebuilt);
}

// ------------------------------------------------------- what cannot be built

#[test]
fn refuses_a_record_that_describes_several_extents() {
  let mut source = sample();
  flag_record(&mut source, "Z.BIN;1", 0x80);

  let err = decode_error(&source, None);

  assert!(
    err.contains("several extents"),
    "the reason should be reported: {err}"
  );
}

#[test]
fn refuses_a_record_that_carries_an_extension_record() {
  let mut source = sample();
  let offset = record_offset(&source, "M.BIN;1");

  source[offset + 1] = 1;

  let err = decode_error(&source, None);

  assert!(
    err.contains("extended attribute"),
    "the reason should be reported: {err}"
  );
}

#[test]
fn refuses_an_interleaved_file() {
  let mut source = sample();
  let offset = record_offset(&source, "M.BIN;1");

  source[offset + 26] = 1;

  let err = decode_error(&source, None);

  assert!(
    err.contains("interleaved"),
    "the reason should be reported: {err}"
  );
}

#[test]
fn refuses_an_image_whose_files_the_codec_cannot_write_back() {
  let mut source = sample();

  // A both-ended field the two halves of which disagree is not something a
  // rebuild could write back as it stands.
  let offset = record_offset(&source, "M.BIN;1");
  source[offset + 6..offset + 10].copy_from_slice(&7u32.to_be_bytes());

  let err = decode_error(&source, None);

  assert!(
    err.contains("one way round"),
    "the reason should be reported: {err}"
  );
}

// ------------------------------------------------- files the image does not hold

/// A mixed-mode disc keeps a file on the track that follows its data track: its
/// record is in the tree, and its contents are in another file altogether.
#[test]
fn keeps_the_record_of_a_file_that_lies_beyond_the_image() {
  let mut source = sample();
  move_beyond_the_image(&mut source, "M.BIN;1", 900, 4096);

  let artifacts = Iso9660.decode(&source, None).unwrap();

  assert!(
    !names(&artifacts).contains(&"M.BIN"),
    "nothing carries the contents of a file outside the image: {}",
    names(&artifacts).join(", ")
  );

  let layout = metadata(&artifacts).layout;
  let beyond = layout
    .directories
    .iter()
    .flat_map(|directory| &directory.records)
    .find(|record| record.name == "M.BIN;1")
    .expect("the record should be preserved");

  assert_eq!(beyond.extent, 900);
  assert_eq!(beyond.length, 4096);

  rebuild(&source);
}

#[test]
fn refuses_to_change_a_file_that_lies_beyond_the_image() {
  let mut source = sample();
  move_beyond_the_image(&mut source, "M.BIN;1", 900, 4096);

  let artifacts = Iso9660.decode(&source, None).unwrap();

  let mut meta = metadata(&artifacts);
  meta.file_order.push("M.BIN".to_owned());

  let mut extended = with_metadata(&artifacts, &meta);
  extended.push(artifact("M.BIN", b"elsewhere"));

  let err = Iso9660.encode(Vec::new(), &extended, None).unwrap_err();

  assert!(
    err.to_string().contains("lie outside the image"),
    "the reason should be reported: {err}"
  );
}

/// An image holding a file whose data lies beyond it may still grow. The blocks
/// past the image's end are pushed along by the blocks it grows by, so the record
/// that reaches them moves with them: the file keeps its distance from the
/// image's end, and stays on the data it described.
#[test]
fn grows_an_image_that_declares_data_it_does_not_hold() {
  let mut source = sample();
  move_beyond_the_image(&mut source, "M.BIN;1", 900, 4096);

  let artifacts = Iso9660.decode(&source, None).unwrap();

  // The image is packed, so a file that outgrows its own blocks takes room past
  // the image's end, and the image grows to hold it.
  let rebuilt = encode(&replacing(&artifacts, "Z.BIN", vec![b'z'; 4096]));

  assert!(
    rebuilt.len() > source.len(),
    "the image should have grown to hold the file"
  );

  let grew = (rebuilt.len() - source.len()) as u32 / BLOCK_SIZE as u32;

  assert!(grew > 0);
  assert_eq!(contents_at(&rebuilt, "Z.BIN"), vec![b'z'; 4096]);

  // The file that lies beyond the image moved by the blocks the image grew by,
  // which is what keeps it 873 blocks past the image's end, as it was.
  let (_, extent, length) = tree(&rebuilt)
    .into_iter()
    .find(|(path, _, _)| path == "M.BIN")
    .expect("the record should still be in the tree");

  assert_eq!(
    extent,
    900 + grew,
    "the record should keep its distance from the image's end"
  );
  assert_eq!(length, 4096);
  assert_eq!(
    extent as usize - rebuilt.len() / BLOCK_SIZE,
    900 - source.len() / BLOCK_SIZE
  );

  // The image declared 27 blocks and the volume declared 27, so the blocks the
  // image gained are the volume's: it ends where the image does.
  assert_eq!(
    both_ended(&rebuilt, DESCRIPTOR_START + 80),
    rebuilt.len() as u32 / BLOCK_SIZE as u32,
    "the volume should be as long as the image"
  );

  // A file that stays within the blocks it held is still written where it was,
  // and the image is not lengthened for it.
  let kept = encode(&replacing(&artifacts, "Z.BIN", b"zz".to_vec()));

  assert_eq!(contents_at(&kept, "Z.BIN"), b"zz".to_vec());
  assert_eq!(kept.len(), source.len());
}

/// A file whose extent runs across the image's end is one run of blocks that a
/// longer image would break in the middle of: inserting blocks there would put
/// new data where the file's own continues. The image refuses to grow for it and
/// names the file.
#[test]
fn refuses_to_grow_an_image_whose_record_straddles_its_end() {
  let mut source = sample();

  // A block of room after the filesystem, then a file that claims that block and
  // the one past the image's end: two blocks of it inside the image, two out.
  source.extend(std::iter::repeat_n(0u8, BLOCK_SIZE));
  move_beyond_the_image(&mut source, "M.BIN;1", 27, 4096);

  let artifacts = Iso9660.decode(&source, None).unwrap();
  let err = Iso9660
    .encode(
      Vec::new(),
      &replacing(&artifacts, "Z.BIN", vec![b'z'; 4096]),
      None,
    )
    .unwrap_err()
    .to_string();

  assert!(
    err.contains("M.BIN") && err.contains("may not grow") && err.contains("both sides"),
    "the file that straddles the image's end should be named: {err}"
  );

  // A file that stays within the blocks it held is still written: nothing has
  // to move for it, so nothing past the image's end has to either.
  let kept = encode(&replacing(&artifacts, "Z.BIN", b"zz".to_vec()));

  assert_eq!(contents_at(&kept, "Z.BIN"), b"zz".to_vec());
}

/// A volume may declare a DVD's worth of blocks, far past what a compact disc
/// holds, and an image that declares one grows the same way: the size it states
/// moves along with the blocks past its end, and nothing in the codec caps it at
/// the size of the medium it was read from.
#[test]
fn grows_an_image_whose_volume_declares_blocks_past_a_compact_disc() {
  let mut source = sample();

  // Two million four hundred thousand blocks of 2048 bytes is 4.6 GiB: the
  // volume of a single-layer DVD, with a file sitting near its far end.
  declare_volume_space_size(&mut source, 2_400_000);
  move_beyond_the_image(&mut source, "M.BIN;1", 2_399_000, 4096);

  let artifacts = Iso9660.decode(&source, None).unwrap();
  let rebuilt = encode(&replacing(&artifacts, "Z.BIN", vec![b'z'; 4096]));

  assert!(rebuilt.len() > source.len(), "the image should have grown");

  let grew = (rebuilt.len() - source.len()) as u32 / BLOCK_SIZE as u32;

  assert_eq!(
    both_ended(&rebuilt, DESCRIPTOR_START + 80),
    2_400_000 + grew,
    "the volume should state the size those blocks moved to"
  );

  let (_, extent, _) = tree(&rebuilt)
    .into_iter()
    .find(|(path, _, _)| path == "M.BIN")
    .expect("the record should still be in the tree");

  assert_eq!(extent, 2_399_000 + grew);
}

/// Writing over bytes the codec does not interpret would lose data the image
/// holds, so a rebuild refuses rather than guesses.
#[test]
fn refuses_a_layout_that_would_write_over_the_bytes_it_preserves() {
  let mut source = sample();
  source.extend(std::iter::repeat_n(0x5Au8, BLOCK_SIZE));

  let artifacts = Iso9660.decode(&source, None).unwrap();

  // A hand edit that points opaque bytes at a file's own extent: a rebuild
  // cannot write both, and says so rather than choosing.
  let mut meta = metadata(&artifacts);
  meta.layout.opaque.push(super::metadata::Opaque {
    offset: both_ended(&source, 156 + 2) as u64 * BLOCK_SIZE as u64,
    data: super::metadata::Hex::from(&b"over the tree"[..]),
  });

  let err = Iso9660
    .encode(Vec::new(), &with_metadata(&artifacts, &meta), None)
    .unwrap_err();

  assert!(
    err.to_string().contains("does not interpret"),
    "the reason should be reported: {err}"
  );
}

/// A decode never describes two files over the same blocks — it refuses two
/// structures in the same bytes — and no placement policy moves one there: the
/// policies keep every region clear of the others. A hand-edited layout can
/// still point two records at one extent, and a rebuild writing the two in
/// turn would leave the image holding only the second, silently. The rebuild
/// says which file it would have written over instead. An empty file writes no
/// bytes wherever its record points, so one sharing an extent is not a
/// collision.
#[test]
fn refuses_a_layout_that_writes_one_file_over_another() {
  let artifacts = Iso9660.decode(&ordered(), None).unwrap();

  let (first, _) = extent_of(&ordered(), "FIRST.BIN");

  // SECOND.BIN's record moved on to the blocks FIRST.BIN already fills: both
  // keep the length they had, so nothing but the extent disagrees.
  let mut meta = metadata(&artifacts);

  let second = meta
    .layout
    .directories
    .iter_mut()
    .flat_map(|directory| &mut directory.records)
    .find(|record| record.is_file() && record.artifact_name() == "SECOND.BIN")
    .expect("the fixture should hold SECOND.BIN");

  second.extent = first;

  let err = Iso9660
    .encode(Vec::new(), &with_metadata(&artifacts, &meta), None)
    .unwrap_err()
    .to_string();

  assert!(
    err.contains("SECOND.BIN") && err.contains("FIRST.BIN"),
    "the file and the one it would be written over should both be named: {err}"
  );

  // The same extent under an empty file is no collision: the record is
  // written, and the blocks it names hold the file that already fills them.
  let shrunk = replacing(&artifacts, "SECOND.BIN", Vec::new());
  let mut meta = metadata(&artifacts);

  let second = meta
    .layout
    .directories
    .iter_mut()
    .flat_map(|directory| &mut directory.records)
    .find(|record| record.is_file() && record.artifact_name() == "SECOND.BIN")
    .expect("the fixture should hold SECOND.BIN");

  second.extent = first;

  let rebuilt = Iso9660
    .encode(Vec::new(), &with_metadata(&shrunk, &meta), None)
    .expect("an empty file at a taken extent writes nothing there and should rebuild");

  assert_eq!(
    contents_at(&rebuilt, "FIRST.BIN"),
    vec![b'a'; BLOCK_SIZE * 2],
    "the file that fills the blocks should have kept its contents"
  );
}

#[test]
fn drops_the_files_that_lie_beyond_the_image_when_asked() {
  let mut source = sample();
  move_beyond_the_image(&mut source, "M.BIN;1", 900, 4096);

  let artifacts = Iso9660
    .decode(&source, Some(r#"{"out_of_bounds_entries":"ignore"}"#))
    .unwrap();

  assert!(
    !metadata(&artifacts)
      .layout
      .directories
      .iter()
      .flat_map(|directory| &directory.records)
      .any(|record| record.name == "M.BIN;1"),
    "the record should have been dropped"
  );

  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  assert!(
    !read_back(&rebuilt).iter().any(|(path, _)| path == "M.BIN"),
    "the rebuilt image should not hold the dropped file"
  );
}

#[test]
fn refuses_the_files_that_lie_beyond_the_image_when_asked() {
  let mut source = sample();
  move_beyond_the_image(&mut source, "M.BIN;1", 900, 4096);

  let err = decode_error(&source, Some(r#"{"out_of_bounds_entries":"error"}"#));

  assert!(err.contains("M.BIN"), "the file should be named: {err}");
}

// -------------------------------------------------------------- other writers

#[test]
fn reads_the_iso_9660_names_of_an_image_carrying_both_extensions() {
  // The shape `mkisofs -R -J` produces. Rock Ridge names these files
  // `lower.bin` and `noextension` and holds `LOWER.BIN;1` and `NOEXTENS.;1`
  // beside them, and what this codec reads has to be the latter: a rebuild
  // keeps the identifiers the image holds.
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

  // Rock Ridge keeps its own names in the records' system use areas, and a
  // supplementary descriptor holds the Joliet tree. Both are preserved as they
  // stand, which is what lets this image round trip at all.
  rebuild(&source);
}

#[test]
fn refuses_to_move_a_file_whose_record_carries_a_system_use_area() {
  let Some(source) = mkisofs(
    "ROCKRIDGE",
    &[
      ("lower.bin", &[b'a'; 4096]),
      ("zulu.bin", b"z"),
    ],
    &["-R"],
  ) else {
    eprintln!("skipping: mkisofs is not installed");
    return;
  };

  let artifacts = Iso9660.decode(&source, None).unwrap();
  let layout = &metadata(&artifacts).layout;
  let extent_of = |identifier: &str| {
    layout
      .directories
      .iter()
      .flat_map(|directory| &directory.records)
      .find(|record| record.name == identifier)
      .map(|record| record.extent)
  };

  assert!(
    extent_of("ZULU.BIN;1") > extent_of("LOWER.BIN;1"),
    "the fixture should hold a file after the one this test grows"
  );

  // Rock Ridge keeps its entries in the records' system use areas, which this
  // codec preserves without interpreting. It will not assume that an area which
  // said where a file sat still says it after the file moves, so a file that
  // outgrows the room that follows it is refused rather than moved.
  let err = Iso9660
    .encode(
      Vec::new(),
      &replacing(&artifacts, "LOWER.BIN", vec![b'a'; 4_000_000]),
      None,
    )
    .unwrap_err();

  assert!(
    err.to_string().contains("LOWER.BIN"),
    "the file should be named: {err}"
  );
  assert!(
    err.to_string().contains("system use area"),
    "the reason should be reported: {err}"
  );

  // Contents that take the room it already has replace it where it is, which
  // leaves the area describing what it described.
  let held = contents_at(&source, "LOWER.BIN");

  assert_eq!(held.len(), 4096);
  assert_eq!(
    Iso9660
      .encode(Vec::new(), &replacing(&artifacts, "LOWER.BIN", held), None)
      .unwrap(),
    source
  );
}

#[test]
fn refuses_to_grow_a_file_whose_image_carries_a_supplementary_descriptor() {
  // Joliet is a second tree over the same files, whose records state each file's
  // extent and length. This codec keeps those bytes as they stand, so it refuses
  // a length change rather than leave that tree stating a length the file no
  // longer has — even where the file would have grown in place.
  let Some(source) = mkisofs_both_extensions("BOTH", &[("lower.bin", &[b'a'; 4096])]) else {
    eprintln!("skipping: mkisofs is not installed");
    return;
  };

  let artifacts = Iso9660.decode(&source, None).unwrap();

  let err = Iso9660
    .encode(
      Vec::new(),
      &replacing(&artifacts, "LOWER.BIN", vec![b'a'; 4_000_000]),
      None,
    )
    .unwrap_err();

  assert!(
    err.to_string().contains("LOWER.BIN"),
    "the file should be named: {err}"
  );
  assert!(
    err.to_string().contains("descriptor"),
    "the reason should be reported: {err}"
  );
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

  // What cdrtools laid out, this codec rebuilds byte for byte: the records, the
  // path tables, the descriptor and the area it filled in it all come back as
  // they were.
  rebuild(&source);
}

#[test]
fn reads_back_what_it_writes_through_a_reader_that_is_not_hadris() {
  let rebuilt = rebuild(&sample());

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

// -------------------------------------------------------- placement policies

/// Rebuilds a decode under one placement policy.
fn encode_under(artifacts: &[DecodedArtifact], placement: &str) -> Vec<u8> {
  Iso9660
    .encode(
      Vec::new(),
      artifacts,
      Some(&format!(r#"{{"placement":"{placement}"}}"#)),
    )
    .unwrap()
}

/// The message a rebuild under one placement policy was refused with.
fn encode_error(artifacts: &[DecodedArtifact], placement: &str) -> String {
  match Iso9660.encode(
    Vec::new(),
    artifacts,
    Some(&format!(r#"{{"placement":"{placement}"}}"#)),
  ) {
    Ok(image) => panic!(
      "the rebuild should have been refused, and it built {} blocks",
      image.len() / BLOCK_SIZE
    ),
    Err(err) => err.to_string(),
  }
}

/// Where one file sits in an image, and how long it is.
fn extent_of(image: &[u8], path: &str) -> (u32, u32) {
  tree(image)
    .into_iter()
    .find(|(held, _, _)| held == path)
    .map(|(_, extent, length)| (extent, length))
    .unwrap_or_else(|| panic!("the image should hold '{path}'"))
}

/// The block after the last region an image lays out: its files, its
/// directories, and the two path tables its descriptor names.
///
/// A Hadris image ends where its path tables do, and nothing sits between them
/// and the image's end, so an image whose `regions_end` is its own length is one
/// that a rebuild lays out to the very end of it.
fn regions_end(image: &[u8]) -> u32 {
  let mut end = 0;

  for (_, extent, length) in tree(image) {
    end = end.max(extent + length.div_ceil(BLOCK_SIZE as u32));
  }

  for (_, block) in directory_blocks(image) {
    let at = block as usize * BLOCK_SIZE;
    let length = both_ended(image, at + 10);

    end = end.max(block + length.div_ceil(BLOCK_SIZE as u32));
  }

  let blocks = both_ended(image, DESCRIPTOR_START + 132).div_ceil(BLOCK_SIZE as u32);

  for (offset, big_endian) in [
    (140usize, false),
    (148, true),
  ] {
    let bytes = image[DESCRIPTOR_START + offset..DESCRIPTOR_START + offset + 4]
      .try_into()
      .expect("four bytes");
    let sector = if big_endian {
      u32::from_be_bytes(bytes)
    } else {
      u32::from_le_bytes(bytes)
    };

    end = end.max(sector + blocks);
  }

  end
}

/// The Volume Space Size an image declares.
fn volume_space_size(image: &[u8]) -> u32 {
  both_ended(image, DESCRIPTOR_START + 80)
}

/// The directories an image's path table names, with the block each one starts
/// at, read out of the table itself.
fn path_table_blocks(image: &[u8]) -> Vec<(String, u32)> {
  let size = both_ended(image, DESCRIPTOR_START + 132) as usize;
  let sector = u32::from_le_bytes(
    image[DESCRIPTOR_START + 140..DESCRIPTOR_START + 144]
      .try_into()
      .expect("four bytes"),
  );
  let start = sector as usize * BLOCK_SIZE;
  let table = &image[start..start + size];

  let mut found = Vec::new();
  let mut offset = 0;

  while offset < size {
    let name_len = table[offset] as usize;
    let extent = u32::from_le_bytes(
      table[offset + 2..offset + 6]
        .try_into()
        .expect("four bytes"),
    );

    // The root's identifier is a single zero byte.
    let name = match &table[offset + 8..offset + 8 + name_len] {
      [0] => String::new(),
      name => String::from_utf8_lossy(name).into_owned(),
    };

    found.push((name, extent));
    offset += 8 + name_len + usize::from(!name_len.is_multiple_of(2));
  }

  found.sort();
  found
}

/// Each directory an image holds, with the block its records sit at: the root
/// from the record in the descriptor, and every other one from the record its
/// parent holds for it, keyed by the identifier its parent names it by — which
/// is how a path table names it too. (For the shallow fixtures that was its
/// full path as well; an image nesting its directories tells them apart.)
fn directory_blocks(image: &[u8]) -> Vec<(String, u32)> {
  let mut found = vec![(String::new(), both_ended(image, DESCRIPTOR_START + 156 + 2))];

  walk_records(image, |_, _, extent, _, is_directory, identifier| {
    let bytes = identifier.as_bytes();

    // The entries a directory holds for itself and for its parent name a
    // directory that has already been walked to reach them.
    if is_directory && !(bytes.len() == 1 && bytes[0] < 2) {
      found.push((identifier.to_owned(), extent));
    }
  });

  found.sort();
  found
}

/// Every file and directory an image holds, with the block it starts at.
fn regions_of(image: &[u8]) -> Vec<(String, u32)> {
  let mut found = blocks(image);

  found.extend(directory_blocks(image));
  found.sort();

  found
}

/// Asserts that an image's path table names the directories its records name,
/// and that each directory's records really are where the two of them say.
///
/// The path table and the records are two lists of the same blocks, written in
/// different places: a rebuild that re-laid out the image has to move both.
fn assert_directories_agree(image: &[u8]) {
  let named = directory_blocks(image);

  assert_eq!(
    path_table_blocks(image),
    named,
    "the path table should name the directories the records name"
  );

  for (path, block) in named {
    let at = block as usize * BLOCK_SIZE;
    let name = if path.is_empty() { "/" } else { &path };

    assert_eq!(
      both_ended(image, at + 2),
      block,
      "the directory '{name}' should hold its own records at the block it claims"
    );
    assert!(
      both_ended(image, at + 10) > 0,
      "the directory '{name}' should declare a length"
    );
  }
}

/// Three files of two blocks each, in the order they sit: the sequence a policy
/// that re-lays out the image has to keep.
fn ordered() -> Vec<u8> {
  image(vec![
    file("FIRST.BIN", &[b'a'; BLOCK_SIZE * 2]),
    file("SECOND.BIN", &[b'b'; BLOCK_SIZE * 2]),
    file("THIRD.BIN", &[b'c'; BLOCK_SIZE * 2]),
  ])
}

/// Whatever the policy, an image no file changed in goes back exactly as it
/// was: nothing has to move for a file that still fits where it was, so nothing
/// does.
#[test]
fn every_placement_policy_rebuilds_an_unchanged_image_byte_for_byte() {
  for source in [ordered(), sample()] {
    let artifacts = Iso9660.decode(&source, None).unwrap();

    for placement in [
      "shift",
      "relocate",
      "preserve",
    ] {
      assert_same(
        &encode_under(&artifacts, placement),
        &source,
        &format!("an unchanged image should come back byte for byte under '{placement}'"),
      );
    }
  }
}

/// A manifest written before placement policies existed records no args at all,
/// and one written by hand may name none either. Both keep the planner the codec
/// planned with before, rather than quietly re-laying out an image that was
/// added under the older behaviour; a decode added now is recorded with the
/// policy the codec defaults to.
#[test]
fn args_that_name_no_placement_keep_the_planner_the_codec_had() {
  let source = ordered();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let (first, _) = extent_of(&source, "FIRST.BIN");
  let (second, _) = extent_of(&source, "SECOND.BIN");

  let grown = replacing(
    &artifacts,
    "FIRST.BIN",
    vec![b'a'; (second - first + 4) as usize * BLOCK_SIZE],
  );

  assert_eq!(
    Iso9660.encode(Vec::new(), &grown, Some("{}")).unwrap(),
    encode_under(&grown, "relocate"),
    "args that name no placement should plan the way the codec did before there were policies"
  );

  // The args a decode added now is recorded with, which is what a manifest
  // written from here on carries.
  let recorded = r#"{"placement":"shift"}"#;
  let rebuilt = Iso9660.encode(Vec::new(), &grown, Some(recorded)).unwrap();

  assert_eq!(
    extent_of(&rebuilt, "FIRST.BIN").0,
    first,
    "the recorded policy should keep the file that grew where it was"
  );
  assert_eq!(extent_of(&rebuilt, "SECOND.BIN").0, second + 4);
}

/// The point of re-laying the image out in its own order: a file that grows past
/// the blocks it held keeps its extent, and everything after it moves along by
/// exactly the blocks its growth covered.
#[test]
fn shift_pushes_what_follows_a_file_that_grew() {
  let source = ordered();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let (first, _) = extent_of(&source, "FIRST.BIN");
  let (second, _) = extent_of(&source, "SECOND.BIN");
  let (third, _) = extent_of(&source, "THIRD.BIN");

  let before = (source.len() / BLOCK_SIZE) as u32;

  assert_eq!(
    regions_end(&source),
    before,
    "the fixture should end with the last region it lays out"
  );

  // Four blocks more than the room the next file leaves: the push is exactly
  // those four blocks, and the image grows to hold the last of them.
  let pushed = 4;
  let grown = vec![b'a'; ((second - first + pushed) as usize) * BLOCK_SIZE];
  let rebuilt = encode_under(&replacing(&artifacts, "FIRST.BIN", grown.clone()), "shift");

  assert_eq!(
    extent_of(&rebuilt, "FIRST.BIN"),
    (first, grown.len() as u32),
    "a file that grew should keep the extent it had"
  );
  assert_eq!(contents_at(&rebuilt, "FIRST.BIN"), grown);

  assert_eq!(extent_of(&rebuilt, "SECOND.BIN").0, second + pushed);
  assert_eq!(extent_of(&rebuilt, "THIRD.BIN").0, third + pushed);
  assert_eq!(
    contents_at(&rebuilt, "SECOND.BIN"),
    vec![b'b'; BLOCK_SIZE * 2]
  );
  assert_eq!(
    contents_at(&rebuilt, "THIRD.BIN"),
    vec![b'c'; BLOCK_SIZE * 2]
  );

  assert_eq!(
    (rebuilt.len() / BLOCK_SIZE) as u32,
    before + pushed,
    "the image should grow by the blocks the pushed regions ran past its end"
  );

  // The image keeps its order, and the program that seeks to a block of its own
  // still finds it: only the files after the grown one moved, and they all moved
  // together.
  assert_eq!(
    on_disc(&rebuilt),
    on_disc(&source),
    "the files should sit in the order they sat in"
  );
  assert_eq!(
    blocks(&rebuilt)
      .into_iter()
      .filter(|(path, _)| path == "FIRST.BIN")
      .collect::<Vec<_>>(),
    blocks(&source)
      .into_iter()
      .filter(|(path, _)| path == "FIRST.BIN")
      .collect::<Vec<_>>()
  );

  // And every reference the image holds moved with them.
  assert_directories_agree(&rebuilt);

  let mut expected = read_back(&source);

  for (path, data) in &mut expected {
    if path == "FIRST.BIN" {
      *data = grown.clone();
    }
  }

  assert_eq!(read_back(&rebuilt), expected);
}

/// Two files that grow push the regions after them cumulatively: each one shifts
/// the tail by what it needs and no more, so the image keeps every region's place
/// in the sequence.
#[test]
fn shift_pushes_cumulatively_for_every_file_that_grows() {
  let source = ordered();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let (first, _) = extent_of(&source, "FIRST.BIN");
  let (second, _) = extent_of(&source, "SECOND.BIN");
  let (third, _) = extent_of(&source, "THIRD.BIN");

  let before = (source.len() / BLOCK_SIZE) as u32;

  // The first file grows four blocks past the room before the second, and the
  // third grows six blocks past its own: the second moves by the first file's
  // four, and the third by those four and its own six together.
  let grown = replacing(
    &replacing(
      &artifacts,
      "FIRST.BIN",
      vec![b'a'; (second - first + 4) as usize * BLOCK_SIZE],
    ),
    "THIRD.BIN",
    vec![b'c'; BLOCK_SIZE * 8],
  );

  let rebuilt = encode_under(&grown, "shift");

  assert_eq!(extent_of(&rebuilt, "FIRST.BIN").0, first);
  assert_eq!(extent_of(&rebuilt, "SECOND.BIN").0, second + 4);
  assert_eq!(
    extent_of(&rebuilt, "THIRD.BIN").0,
    third + 4,
    "the third file should keep its distance from the second"
  );
  assert_eq!(extent_of(&rebuilt, "THIRD.BIN").1, (BLOCK_SIZE * 8) as u32);
  assert_eq!(
    (rebuilt.len() / BLOCK_SIZE) as u32,
    before + 4 + 6,
    "the image should hold the four blocks the second file moved and the six the third grew by"
  );

  assert_eq!(on_disc(&rebuilt), on_disc(&source));
  assert_directories_agree(&rebuilt);
}

/// A file that shrank keeps the extent it held: re-laying out the image is what a
/// file that grew asks for, not what one that gave room back does. The blocks it
/// no longer fills are left as they are, which is what a reader that seeks to a
/// block of another file relies on.
#[test]
fn shift_keeps_a_file_that_shrank_where_it_was() {
  let source = ordered();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let rebuilt = encode_under(
    &replacing(&artifacts, "SECOND.BIN", vec![b'b'; BLOCK_SIZE]),
    "shift",
  );

  assert_eq!(
    blocks(&rebuilt),
    blocks(&source),
    "nothing should move for a file that gave room back"
  );
  assert_eq!(extent_of(&rebuilt, "SECOND.BIN").1, BLOCK_SIZE as u32);
  assert_eq!(contents_at(&rebuilt, "SECOND.BIN"), vec![b'b'; BLOCK_SIZE]);
  assert_eq!(
    rebuilt.len(),
    source.len(),
    "the image should not be re-laid out for it"
  );
}

/// A directory is named by its own record, by the record its parent holds for it,
/// and by a path table entry, so a rebuild that pushes one along has to state
/// where it went in all three places.
///
/// A directory grows in place and a Hadris image lays every directory out before
/// its files, so the way to push one is to grow the directory ahead of it: enough
/// new records at the root and the root takes the block the next directory sits in.
#[test]
fn shift_moves_a_directory_and_every_reference_to_it() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let before = regions_of(&source);
  let root = directory_blocks(&source)
    .into_iter()
    .find(|(path, _)| path.is_empty())
    .map(|(_, block)| block)
    .expect("the fixture should hold a root directory");

  // Enough files for the root's records to need a second block, which is the
  // block the directory 'A' holds.
  let mut extended = copy(&artifacts);
  let mut meta = metadata(&artifacts);
  let added = 48;

  for index in 0..added {
    let name = format!("ADDED{index:02}.BIN");

    meta.file_order.push(name.clone());
    extended.push(artifact(&name, b"x"));
  }

  let rebuilt = encode_under(&with_metadata(&extended, &meta), "shift");

  // The root took the block that followed it, and every region that followed the
  // root moved by one block, the files and the path tables with them.
  assert_eq!(
    both_ended(&rebuilt, root as usize * BLOCK_SIZE + 10),
    (BLOCK_SIZE * 2) as u32,
    "the root's records should have taken a second block"
  );
  assert_eq!(
    directory_blocks(&rebuilt)
      .into_iter()
      .find(|(path, _)| path.is_empty())
      .map(|(_, block)| block),
    Some(root),
    "the directory that grew should keep its extent"
  );
  assert_eq!(
    regions_of(&rebuilt).len(),
    before.len() + added as usize,
    "every file should still be there"
  );

  for (path, block) in before.iter().filter(|(_, block)| *block > root) {
    let moved = regions_of(&rebuilt)
      .into_iter()
      .find(|(held, _)| held == path)
      .map(|(_, block)| block)
      .expect("the rebuilt image should hold what the source did");

    assert_eq!(
      moved,
      block + 1,
      "'{path}' should move by the blocks the grown directory took"
    );
  }

  // The directories' own records, their parents' records and the path table
  // entries all state where they went, and the records and the table agree.
  assert_directories_agree(&rebuilt);

  // The files the rebuild added are readable, and so is every file the image had.
  for index in 0..added {
    let name = format!("ADDED{index:02}.BIN");

    assert_eq!(contents_at(&rebuilt, &name), b"x".to_vec());
  }

  assert_eq!(read_back(&rebuilt).len(), added as usize + 4);
}

/// The records that describe data beyond the image are carried by the blocks the
/// image grew by, whichever policy grew it: the file stays at the same distance
/// from the image's end, on the data it described.
#[test]
fn shift_carries_the_records_that_describe_data_beyond_the_image() {
  let mut source = ordered();
  move_beyond_the_image(&mut source, "THIRD.BIN;1", 900, 4096);

  let artifacts = Iso9660.decode(&source, None).unwrap();

  let (first, _) = extent_of(&source, "FIRST.BIN");
  let (second, _) = extent_of(&source, "SECOND.BIN");
  let third = 900;

  let pushed = 4;
  let grown = vec![b'a'; (second - first + pushed) as usize * BLOCK_SIZE];
  let rebuilt = encode_under(&replacing(&artifacts, "FIRST.BIN", grown), "shift");

  let grew = ((rebuilt.len() - source.len()) / BLOCK_SIZE) as u32;

  assert_eq!(
    grew, pushed,
    "the image should grow by what the push ran past it"
  );
  assert_eq!(extent_of(&rebuilt, "SECOND.BIN").0, second + pushed);
  assert_eq!(extent_of(&rebuilt, "THIRD.BIN").0, third + grew);
  assert_eq!(extent_of(&rebuilt, "THIRD.BIN").1, 4096);
  assert_eq!(
    extent_of(&rebuilt, "THIRD.BIN").0 - (rebuilt.len() / BLOCK_SIZE) as u32,
    extent_of(&source, "THIRD.BIN").0 - (source.len() / BLOCK_SIZE) as u32,
    "the file the image does not hold should keep its distance from its end"
  );

  // The image declared exactly as many blocks as it held, so the blocks it
  // gained are the volume's: it ends where the image does.
  assert_eq!(
    volume_space_size(&rebuilt),
    (rebuilt.len() / BLOCK_SIZE) as u32
  );
  assert_directories_agree(&rebuilt);
}

/// `relocate` is what the codec did before it had policies, and it is what it
/// still does when asked for it: the one file that no longer fits moves, and
/// nothing else does.
#[test]
fn relocate_moves_the_one_file_that_no_longer_fits() {
  let source = image(vec![
    file("FIRST.BIN", &[b'a'; BLOCK_SIZE * 2]),
    file("SECOND.BIN", &[b'b'; BLOCK_SIZE * 2]),
  ]);
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let (first, _) = extent_of(&source, "FIRST.BIN");
  let (second, _) = extent_of(&source, "SECOND.BIN");

  let grown = vec![b'a'; (second - first + 4) as usize * BLOCK_SIZE];
  let rebuilt = encode_under(
    &replacing(&artifacts, "FIRST.BIN", grown.clone()),
    "relocate",
  );

  assert_eq!(
    extent_of(&rebuilt, "SECOND.BIN"),
    extent_of(&source, "SECOND.BIN"),
    "the file that still fits should not move for another"
  );
  assert_ne!(
    extent_of(&rebuilt, "FIRST.BIN").0,
    first,
    "the file that no longer fits should be moved out of the way"
  );
  assert_eq!(contents_at(&rebuilt, "FIRST.BIN"), grown);
  assert_eq!(
    contents_at(&rebuilt, "SECOND.BIN"),
    vec![b'b'; BLOCK_SIZE * 2]
  );

  // The file went to room past the image's end, since the image leaves none.
  assert!(rebuilt.len() > source.len());
  assert!(extent_of(&rebuilt, "FIRST.BIN").0 >= second);
  assert_directories_agree(&rebuilt);
}

/// `preserve` moves nothing: a change is written where the file already is when
/// it fits the room the file holds or the free blocks that follow it, and refused
/// otherwise rather than answered by moving a file or shifting one.
#[test]
fn preserve_keeps_every_extent_where_it_is() {
  let source = ordered();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let (first, _) = extent_of(&source, "FIRST.BIN");
  let (second, _) = extent_of(&source, "SECOND.BIN");

  // Contents that take the room the file already has replace it where it is.
  let kept = encode_under(
    &replacing(&artifacts, "SECOND.BIN", vec![b'b'; BLOCK_SIZE]),
    "preserve",
  );

  assert_eq!(blocks(&kept), blocks(&source));
  assert_eq!(contents_at(&kept, "SECOND.BIN"), vec![b'b'; BLOCK_SIZE]);

  // One that needs the blocks that follow it is refused: they are taken, and
  // this policy will not move the file that holds them.
  let grown = replacing(
    &artifacts,
    "FIRST.BIN",
    vec![b'a'; (second - first + 1) as usize * BLOCK_SIZE],
  );
  let err = encode_error(&grown, "preserve");

  assert!(
    err.contains("FIRST.BIN") && err.contains("preserve"),
    "the file and the policy should be named: {err}"
  );

  // A file with free room behind it takes that room and nothing else: this is the
  // case the policy exists for, a change written where the file already sits. The
  // fixture keeps its last file in the room the codec left behind the path tables,
  // with free blocks after it.
  let mut room = ordered();

  room.extend(std::iter::repeat_n(0u8, BLOCK_SIZE * 4));
  move_beyond_the_image(&mut room, "THIRD.BIN;1", 27, 4096);

  let room_artifacts = Iso9660.decode(&room, None).unwrap();
  let into_room = encode_under(
    &replacing(&room_artifacts, "THIRD.BIN", vec![b'c'; BLOCK_SIZE * 3]),
    "preserve",
  );

  assert_eq!(extent_of(&into_room, "THIRD.BIN").0, 27);
  assert_eq!(
    contents_at(&into_room, "THIRD.BIN"),
    vec![b'c'; BLOCK_SIZE * 3]
  );
  assert_eq!(
    blocks(&into_room),
    blocks(&room),
    "the file grew into the room behind it, which moved nothing"
  );
  assert_eq!(
    into_room.len(),
    room.len(),
    "the room it grew into was already inside the image"
  );
}

/// A record whose extent runs across the image's end is one run of blocks that a
/// longer image would break in the middle of, whichever policy is asked to
/// lengthen it: the rebuild refuses and names the file rather than write a length
/// its data does not have.
#[test]
fn shift_refuses_to_lengthen_an_image_whose_record_straddles_its_end() {
  let mut source = ordered();

  // A block of room after the filesystem, and a file that claims that block and
  // the one past the image's end: one block of it inside the image, one out.
  source.extend(std::iter::repeat_n(0u8, BLOCK_SIZE));

  let end = (source.len() / BLOCK_SIZE) as u32;

  move_beyond_the_image(&mut source, "THIRD.BIN;1", end - 1, 4096);

  let artifacts = Iso9660.decode(&source, None).unwrap();

  // Nothing grew, so nothing has to be re-laid out: the image comes back as it
  // was.
  assert_same(
    &encode_under(&artifacts, "shift"),
    &source,
    "an image that did not change should not be re-laid out for one that straddles its end",
  );

  let err = encode_error(
    &replacing(&artifacts, "FIRST.BIN", vec![b'a'; BLOCK_SIZE * 8]),
    "shift",
  );

  assert!(
    err.contains("THIRD.BIN") && err.contains("both sides") && err.contains("may not grow"),
    "the file that straddles the image's end should be named: {err}"
  );
}

/// An image carrying a second copy of its path tables keeps that copy as it
/// stands, so a rebuild that would move a directory — or the tables themselves —
/// is refused rather than leave the copy describing where its directories used to
/// be. `shift` moves them for a file that grew; `relocate` moves them for the file
/// that no longer fits.
#[test]
fn refuses_a_move_while_a_second_copy_of_the_path_tables_is_there() {
  let source = sample();
  let artifacts = Iso9660.decode(&source, None).unwrap();

  let mut meta = metadata(&artifacts);
  meta.layout.path_table.optional_l_sector = 1;

  let (z, _) = extent_of(&source, "Z.BIN");
  let (m, _) = extent_of(&source, "M.BIN");

  let grown = replacing(
    &with_metadata(&artifacts, &meta),
    "Z.BIN",
    vec![b'z'; (m - z + 4) as usize * BLOCK_SIZE],
  );

  for placement in ["shift", "relocate"] {
    let err = encode_error(&grown, placement);

    assert!(
      err.contains("path tables"),
      "the copy that would be left behind should be the reason under '{placement}': {err}"
    );
  }
}

/// Rock Ridge keeps its own entries in the records' system use areas, which a
/// rebuild preserves without interpreting. A file whose extent the rebuild keeps
/// leaves those areas describing what they described, so `shift` writes a file
/// that outgrows the room that follows it where `relocate` refuses to move it.
#[test]
fn shift_writes_a_file_relocate_refuses_to_move_for_its_system_use_area() {
  let Some(source) = mkisofs(
    "ROCKRIDGE",
    &[
      ("lower.bin", &[b'a'; 4096]),
      ("zulu.bin", b"z"),
    ],
    &["-R"],
  ) else {
    eprintln!("skipping: mkisofs is not installed");
    return;
  };

  let artifacts = Iso9660.decode(&source, None).unwrap();

  let (lower, _) = extent_of(&source, "LOWER.BIN");
  let (zulu, _) = extent_of(&source, "ZULU.BIN");

  assert!(
    lower < zulu,
    "the fixture should hold a file after this one"
  );

  let grown = vec![b'a'; (zulu - lower + 4) as usize * BLOCK_SIZE];

  let refused = encode_error(
    &replacing(&artifacts, "LOWER.BIN", grown.clone()),
    "relocate",
  );

  assert!(
    refused.contains("system use area"),
    "moving it should be refused for its system use area: {refused}"
  );

  let rebuilt = encode_under(&replacing(&artifacts, "LOWER.BIN", grown.clone()), "shift");

  assert_eq!(extent_of(&rebuilt, "LOWER.BIN").0, lower);
  assert_eq!(contents_at(&rebuilt, "LOWER.BIN"), grown);
  assert!(
    extent_of(&rebuilt, "ZULU.BIN").0 > zulu,
    "the file that followed should have been pushed along"
  );
  assert_directories_agree(&rebuilt);
}

// ------------------------------------- records the decode refuses to accept

/// Two records in one directory describing different extents under one
/// identifier: the image holds one path twice, which the decode refuses by
/// name — the two records sort apart by extent, so only a check on the path
/// itself sees them.
#[test]
fn refuses_two_files_the_image_names_under_the_same_path() {
  let mut source = image(vec![
    file("A.BIN", b"a"),
    file("B.BIN", b"b"),
    file("C.BIN", b"c"),
  ]);

  let extents: Vec<(String, u32)> = tree(&source)
    .into_iter()
    .map(|(path, extent, _)| (path, extent))
    .collect();

  let block = |path: &str| {
    extents
      .iter()
      .find(|(held, _)| held == path)
      .map(|(_, extent)| *extent)
      .unwrap_or_else(|| panic!("the fixture should hold '{path}'"))
  };

  assert!(
    block("A.BIN") < block("B.BIN") && block("B.BIN") < block("C.BIN"),
    "the fixture should sit its files in declaration order, so that the two \n     records this test makes identical are not neighbours in the image"
  );

  // The third record renamed over the first: same identifier length, so the
  // directory's own bytes only change where the name is written.
  let offset = record_offset(&source, "C.BIN;1");
  source[offset + 33..offset + 33 + 7].copy_from_slice(b"A.BIN;1");

  let err = decode_error(&source, None);

  assert!(
    err.contains("two files") && err.contains("A.BIN"),
    "the repeated path should be refused and named: {err}"
  );
}

/// A directory record that starts before a logical block boundary and ends
/// past it. ISO 9660 forbids a record from straddling one, which is why the
/// rebuild pads such a record on to the next block: an image holding one
/// cannot be rebuilt byte for byte, so the decode has to refuse it rather than
/// accept it and let a rebuild silently insert padding under every record that
/// followed it.
#[test]
fn refuses_a_directory_whose_record_straddles_a_block_boundary() {
  let mut source = two_block_root();

  let root = both_ended(&source, DESCRIPTOR_START + 156 + 2) as usize;
  let start = root * BLOCK_SIZE;
  let length = both_ended(&source, start + 10) as usize;

  assert_eq!(
    length,
    BLOCK_SIZE * 2,
    "the fixture's root should hold its records in two blocks"
  );

  // The records of the root, and where the zero byte marking the end of the
  // first block leaves them.
  let mut offsets = Vec::new();
  let mut offset = 0;

  while offset + 34 <= length {
    let size = source[start + offset] as usize;

    if size < 34 {
      offset = (offset.div_euclid(BLOCK_SIZE) + 1) * BLOCK_SIZE;
      continue;
    }

    offsets.push((offset, size));
    offset += size;
  }

  let (next, size) = offsets
    .iter()
    .copied()
    .find(|(at, _)| *at >= BLOCK_SIZE)
    .expect("the fixture should carry a record in the second block");
  let previous_end = offsets
    .iter()
    .copied()
    .filter(|(at, _)| *at < BLOCK_SIZE)
    .map(|(at, size)| at + size)
    .max()
    .expect("the first block should hold records");

  // The marker only stood where the record after it would not fit, so moving
  // that record back over it makes one that runs across the boundary.
  assert!(
    previous_end < BLOCK_SIZE && previous_end + size > BLOCK_SIZE,
    "the record should straddle the boundary once the marker is gone"
  );

  let end = offsets.iter().map(|(at, size)| at + size).max().unwrap();
  let kept = end - next;

  source.copy_within(start + next..start + end, start + previous_end);
  source[start + previous_end + kept..start + end].fill(0);

  let err = decode_error(&source, None);

  assert!(
    err.contains("straddle") || err.contains("boundary"),
    "a record the rebuild would have to move should be refused: {err}"
  );
}

// ------------------------------------------------- descriptors a rebuild keeps

/// A second Primary Volume Descriptor in the descriptor set describes the same
/// volume the first one does: its root record states where the tree sits and
/// its fields state the volume's sizes, and a rebuild keeps its bytes without
/// rewriting them. A change that would move a file or resize one is refused
/// while it is there, the way it is for a supplementary descriptor.
#[test]
fn refuses_to_change_an_image_holding_a_second_primary_descriptor() {
  let mut source = sample();

  // The fixture's set is the primary at block 16 and the terminator at 17,
  // with the root directory at 18. The second primary goes in at 17, the
  // terminator moves to 18, and the root directory moves to a block of its
  // own past the image's end, with every reference to it updated.
  let pvd = source[DESCRIPTOR_START..DESCRIPTOR_START + BLOCK_SIZE].to_vec();
  let root = both_ended(&source, DESCRIPTOR_START + 156 + 2);
  let l_sector = u32::from_le_bytes(pvd[140..144].try_into().expect("four bytes")) as usize;
  let m_sector = u32::from_be_bytes(pvd[148..152].try_into().expect("four bytes")) as usize;

  assert_eq!(root, 18, "the fixture should hold its root at block 18");

  let directory = source[root as usize * BLOCK_SIZE..][..BLOCK_SIZE].to_vec();

  source.extend(std::iter::repeat_n(0u8, BLOCK_SIZE));
  let moved = (source.len() / BLOCK_SIZE - 1) as u32;

  let terminator = {
    let mut block = vec![0u8; BLOCK_SIZE];
    block[0] = 0xFF;
    block[1..6].copy_from_slice(b"CD001");
    block[6] = 1;
    block
  };

  let at = 17 * BLOCK_SIZE;
  source[at..at + BLOCK_SIZE].copy_from_slice(&pvd);

  let at = 18 * BLOCK_SIZE;
  source[at..at + BLOCK_SIZE].copy_from_slice(&terminator);

  let at = moved as usize * BLOCK_SIZE;
  source[at..at + BLOCK_SIZE].copy_from_slice(&directory);

  // The root's new block, stated where the image states it: in the
  // descriptor's record for the root, in the records the root holds for
  // itself and for its parent, and in the two path tables.
  source[DESCRIPTOR_START + 158..DESCRIPTOR_START + 162].copy_from_slice(&moved.to_le_bytes());
  source[DESCRIPTOR_START + 162..DESCRIPTOR_START + 166].copy_from_slice(&moved.to_be_bytes());

  for offset in [at + 2, at + 36] {
    source[offset..offset + 4].copy_from_slice(&moved.to_le_bytes());
    source[offset + 4..offset + 8].copy_from_slice(&moved.to_be_bytes());
  }

  for (sector, big_endian) in [
    (l_sector, false),
    (m_sector, true),
  ] {
    let at = sector * BLOCK_SIZE + 2;

    if big_endian {
      source[at..at + 4].copy_from_slice(&moved.to_be_bytes());
    } else {
      source[at..at + 4].copy_from_slice(&moved.to_le_bytes());
    }
  }

  let artifacts = Iso9660.decode(&source, None).unwrap();

  // The second primary's bytes come back unchanged, so an image nothing
  // changed in still rebuilds byte for byte.
  let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

  assert_same(
    &rebuilt,
    &source,
    "an unchanged rebuild of an image holding a second primary descriptor",
  );

  // But a change would leave its tree stating where each file used to sit, so
  // it is refused rather than written.
  let err = Iso9660
    .encode(
      Vec::new(),
      &replacing(&artifacts, "Z.BIN", vec![b'z'; 9000]),
      None,
    )
    .unwrap_err()
    .to_string();

  assert!(err.contains("volume descriptor"), "{err}");
}

// ------------------------------------------ the descriptor's declared version

/// The File Structure Version names the structure version of the image's
/// records and path tables. It changes no other byte's meaning, so whatever a
/// descriptor declares is carried through instead of validated: the metadata
/// records it, an unchanged rebuild writes it back, and an image declaring the
/// usual 1 leaves it out of the metadata entirely, as it always has.
#[test]
fn round_trips_whatever_file_structure_version_the_descriptor_declares() {
  for version in [1u8, 2, 255] {
    let mut source = sample();

    // BP 882 of the primary descriptor: 16 * 2048 + 881.
    source[DESCRIPTOR_START + 881] = version;

    let artifacts = Iso9660.decode(&source, None).unwrap();
    let declared = metadata(&artifacts).layout.primary.file_structure_version;

    assert_eq!(
      declared, version,
      "the metadata should record version {version}"
    );

    let json = std::str::from_utf8(&artifacts[0].data).unwrap();

    if version == 1 {
      assert!(
        !json.contains("file_structure_version"),
        "the usual version should stay in the default, leaving the metadata as it always read"
      );
    } else {
      assert!(
        json.contains(&format!("\"file_structure_version\": {version}")),
        "metadata declaring version {version} should record it: {json}"
      );
    }

    let rebuilt = Iso9660.encode(Vec::new(), &artifacts, None).unwrap();

    assert_same(
      &rebuilt,
      &source,
      &format!("an unchanged rebuild of an image declaring version {version}"),
    );
  }
}

/// The layout the metadata records has to describe an image the rebuild can
/// hold: a directory past the image's end is refused against the image's own
/// bounds. The rebuild's dimensions come from the declared image length, never
/// from where a directory claims to sit, so moving one cannot inflate any
/// allocation — only draw the refusal.
#[test]
fn refuses_a_layout_whose_directories_lie_past_the_image() {
  let artifacts = Iso9660.decode(&sample(), None).unwrap();

  let mut meta = metadata(&artifacts);

  // Moved past the image's end, with the directory's own record moved along
  // so the layout still describes itself consistently: what is refused is the
  // directory's place, not an inconsistency the edit forgot.
  meta.layout.directories[0].extent = 100;

  let own = meta.layout.directories[0]
    .records
    .iter_mut()
    .find(|record| record.name == "\u{0}")
    .expect("the root holds a record for itself");

  own.extent = 100;

  let err = Iso9660
    .encode(Vec::new(), &with_metadata(&artifacts, &meta), None)
    .unwrap_err()
    .to_string();

  assert!(err.contains("past the end of the image"), "{err}");
}
