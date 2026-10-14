use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use hadris_iso::file::EntryType;
use hadris_iso::read::{IsoImage, PathSeparator};
use hadris_iso::sync::Parsable;
use hadris_iso::volume::VolumeDescriptor;
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
use itertools::Itertools;
use log::*;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::Cursor;
use std::sync::Arc;

mod metadata;

#[cfg(test)]
mod tests;

use metadata::{IsoMetadata, METADATA_NAME};

const CODEC_ID: &str = "std/cdrom/iso9660";
const CODEC_NAME: &str = "ISO 9660 CD-ROM image";
const CODEC_DESC: &str = r#"ISO 9660 CD-ROM image codec

Extracts the files of an ISO 9660 image, preserving its directory hierarchy,
alongside an ISO9660.json artifact describing the image: the Volume Identifier it
carries, and the order its files sit in on the disc. Reassembling rebuilds the
image from the artifacts alone, naming the volume and setting out the files in
the order the metadata records.

That order is a best effort. The writer lays a directory's own files down before
those of the directories it holds, so a file_order that interleaves files across
directories cannot be honoured: the rebuild still holds every file, but reads
them back in an order the metadata will then record instead.

The metadata is never a file of the image, and is always the first artifact a
decode produces, which is where a rebuild looks for it.

Its file_order must name every file artifact exactly once: a file added to or
removed from the image has to be added to or removed from both the manifest's
outputs and the metadata, and a mismatch between the two is refused rather than
guessed at.

Names are read and written as plain ISO 9660 identifiers, and the rebuild is
written as plain ISO 9660 Level 1 with neither Rock Ridge nor Joliet, so that
older readers and emulators can mount it. Level 1 admits a name of up to 8
characters, an extension of up to 3, each drawn from A-Z, 0-9 and _, and
directories take neither an extension nor a version. A name outside that is
refused rather than quietly folded into something that fits, so an image whose
files cannot be expressed in Level 1 cannot be rebuilt.

Files are extracted under their identifiers with the ;1 version suffix removed,
which the rebuild supplies again. The original byte layout is not preserved.

Arguments (JSON):
  out_of_bounds_entries (default: "error")
    How to handle file entries pointing outside the ISO image, as may occur
    with mixed-mode CDs referencing audio tracks.

    error      Reject these entries (default)
    ignore     Skip entries starting beyond the ISO image. They are not
               preserved when rebuilding the image.
    preserve   Planned for a future version

Example:
  {"out_of_bounds_entries":"ignore"}
"#;

impl Codec for Iso9660 {
  fn id(&self) -> &str {
    CODEC_ID
  }

  fn name(&self) -> &str {
    CODEC_NAME
  }

  fn description(&self) -> &str {
    CODEC_DESC
  }

  fn kind(&self) -> CodecKind {
    // A rebuild is built entirely from the artifacts.
    CodecKind::Rebuild
  }

  /// Detects an ISO 9660 image by its Primary Volume Descriptor (PVD).
  ///
  /// The PVD starts at logical sector 16 (offset 0x8000 for 2048-byte sectors).
  /// Its first six bytes must contain:
  ///   - 0x01: Primary Volume Descriptor type.
  ///   - "CD001": ISO 9660 standard identifier (magic bytes).
  ///
  /// A matching header identifies the image as ISO 9660 with high confidence,
  /// without parsing the rest of the volume descriptor.
  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    let Some(offset) = usize::try_from(DESCRIPTOR_SECTOR)
      .ok()
      .and_then(|sector| sector.checked_mul(DESCRIPTOR_SIZE))
    else {
      return Ok(CodecHandlingConfidence::No);
    };

    match data.get(offset..offset + 6) {
      Some(header)
        if header[0] == DESCRIPTOR_TYPE_PRIMARY && &header[1..] == STANDARD_IDENTIFIER =>
      {
        Ok(CodecHandlingConfidence::Certain)
      }
      _ => Ok(CodecHandlingConfidence::No),
    }
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("iso9660".to_owned())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding ISO 9660 image");
    let args = Iso9660CodecArgs::from_args(args)?;

    let image = IsoImage::open(Cursor::new(data))
      .map_err(|err| CodecError::Message(format!("Could not open the ISO 9660 image: {err}")))?;

    let volume_identifier = Self::volume_identifier(data)?;
    let mut entries = Self::walk(
      &image,
      data.len(),
      args.out_of_bounds_entries == OutOfBoundsEntries::Ignore,
    )?;

    entries.sort_by(|left, right| {
      left
        .extent
        .cmp(&right.extent)
        .then_with(|| left.name.cmp(&right.name))
    });

    if let Some(name) = entries.iter().map(|entry| &entry.name).duplicates().next() {
      return Err(CodecError::Message(format!(
        "ISO 9660 image holds two files under the path '{name}'"
      )));
    }

    let metadata = IsoMetadata {
      volume_identifier,
      file_order: entries.iter().map(|entry| entry.name.clone()).collect(),
    };

    // The metadata leads, so the manifest records it first and a rebuild finds
    // it by position.
    let mut artifacts = Vec::with_capacity(entries.len() + 1);
    artifacts.push(DecodedArtifact {
      name: METADATA_NAME.to_owned(),
      data: metadata.to_json()?,
    });

    artifacts.extend(entries.into_iter().map(|entry| DecodedArtifact {
      name: entry.name,
      data: entry.data,
    }));

    Ok(artifacts)
  }

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    _: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding ISO 9660 image");

    // The metadata describes the image rather than belonging to it.
    let (metadata_artifact, files) = artifacts.split_first().ok_or_else(|| {
      CodecError::Message(format!(
        "Expected an ISO metadata artifact, found none among {} artifacts",
        artifacts.len()
      ))
    })?;

    let mut names = BTreeSet::new();
    for artifact in files {
      if !names.insert(artifact.name.as_str()) {
        return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
      }
    }

    let metadata = IsoMetadata::from_json(&metadata_artifact.data)?;
    let ordered = Self::order(&metadata, files)?;

    let mut image = Cursor::new(Vec::new());

    IsoImageWriter::create(
      &mut image,
      Self::input_files(&ordered)?,
      Self::write_options(&metadata.volume_identifier),
    )
    .map_err(|err| CodecError::Message(format!("Could not write the ISO 9660 image: {err}")))?;

    Ok(image.into_inner())
  }
}

/// The logical sector the volume descriptor set starts at.
const DESCRIPTOR_SECTOR: u64 = 16;

/// The size of a single volume descriptor.
const DESCRIPTOR_SIZE: usize = 2048;

/// The descriptor type opening a volume descriptor set.
const DESCRIPTOR_TYPE_PRIMARY: u8 = 0x01;

/// The identifier every ISO 9660 volume descriptor carries.
const STANDARD_IDENTIFIER: &[u8; 5] = b"CD001";

/// The width of a Level 1 file name, and of a directory name.
const LEVEL1_NAME: usize = 8;

/// The width of a Level 1 file extension.
const LEVEL1_EXTENSION: usize = 3;

/// How many directories a walk will open before the tree is taken to be looping.
const MAX_DIRECTORIES: usize = 4096;

pub(crate) struct Iso9660;

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OutOfBoundsEntries {
  #[default]
  Error,
  Ignore,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Iso9660CodecArgs {
  #[serde(default)]
  out_of_bounds_entries: OutOfBoundsEntries,
}

impl LudaredCodecArgs for Iso9660CodecArgs {}

struct IsoFileEntry {
  name: String,
  data: Vec<u8>,
  /// Extent numbers count the whole image, so they order the files by where they
  /// physically sit rather than by the order directories list their entries in.
  extent: u32,
}

impl Iso9660 {
  /// Validates an ISO 9660 Level 1 directory name.
  ///
  /// Must be non-empty, at most 8 characters, contain only A-Z, 0-9 and _,
  /// and have no extension.
  fn validate_directory_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
      return Err("directory name is empty".to_owned());
    }

    if name.len() > LEVEL1_NAME {
      return Err(format!(
        "directory name '{name}' exceeds {LEVEL1_NAME} characters"
      ));
    }

    if name.contains('.') {
      return Err("a directory name takes no extension".to_owned());
    }

    if !name
      .bytes()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
      return Err(format!(
        "directory name '{name}' may contain only A-Z, 0-9 and _"
      ));
    }

    Ok(())
  }

  /// Validates an ISO 9660 Level 1 file name.
  ///
  /// Must have a non-empty base name of at most 8 characters and an extension
  /// of at most 3 characters. Both may contain only A-Z, 0-9 and _.
  fn validate_file_name(name: &str) -> Result<(), String> {
    let (base, extension) = name.split_once('.').unwrap_or((name, ""));

    if base.is_empty() {
      return Err("file name is empty".to_owned());
    }

    if base.len() > LEVEL1_NAME {
      return Err(format!(
        "file name '{base}' exceeds {LEVEL1_NAME} characters"
      ));
    }

    if !base
      .bytes()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
      return Err(format!(
        "file name '{base}' may contain only A-Z, 0-9 and _"
      ));
    }

    if extension.len() > LEVEL1_EXTENSION {
      return Err(format!(
        "file extension '{extension}' exceeds {LEVEL1_EXTENSION} characters"
      ));
    }

    if !extension
      .bytes()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
      return Err(format!(
        "file extension '{extension}' may contain only A-Z, 0-9 and _"
      ));
    }

    Ok(())
  }

  /// The Volume Identifier recorded in the Primary Volume Descriptor.
  ///
  /// Only the padding ISO 9660 mandates is removed. A value that is not text is
  /// refused rather than mangled into something that fits.
  fn volume_identifier(data: &[u8]) -> Result<String, CodecError> {
    let start = usize::try_from(DESCRIPTOR_SECTOR)
      .ok()
      .and_then(|sector| sector.checked_mul(DESCRIPTOR_SIZE))
      .ok_or_else(|| {
        CodecError::Message("ISO 9660 image is too small to hold a descriptor set".to_owned())
      })?;

    let descriptor = data.get(start..start + DESCRIPTOR_SIZE).ok_or_else(|| {
      CodecError::Message(format!(
        "ISO 9660 volume descriptor set is truncated (Minimum: {} bytes)",
        start + DESCRIPTOR_SIZE
      ))
    })?;

    if descriptor.first() != Some(&DESCRIPTOR_TYPE_PRIMARY)
      || &descriptor[1..6] != STANDARD_IDENTIFIER.as_slice()
    {
      return Err(CodecError::Message(
        "ISO 9660 primary volume descriptor is missing or malformed".to_owned(),
      ));
    }

    let descriptor = <VolumeDescriptor as Parsable>::parse(&mut &descriptor[..])
      .map_err(|err| CodecError::Message(format!("Could not read the volume descriptor: {err}")))?;

    let VolumeDescriptor::Primary(primary) = descriptor else {
      return Err(CodecError::Message(
        "ISO 9660 volume descriptor set does not begin with a primary volume descriptor".to_owned(),
      ));
    };

    // `len` is where the field stops being padding, so it is the length of the
    // value rather than the width of the field.
    let significant = &primary.volume_identifier.as_bytes()[..primary.volume_identifier.len()];

    std::str::from_utf8(significant)
      .map(str::to_owned)
      .map_err(|err| {
        CodecError::Message(format!(
          "ISO 9660 volume identifier is not valid UTF-8 and cannot be recorded: {err}"
        ))
      })
  }

  fn walk(
    image: &IsoImage<Cursor<&[u8]>>,
    image_len: usize,
    ignore_out_of_bounds: bool,
  ) -> Result<Vec<IsoFileEntry>, CodecError> {
    let root = Self::root_of(image)?;
    let mut entries = Vec::new();
    let mut pending = vec![(root.iter(image), String::new())];
    let mut visited = 0;

    while let Some((directory, prefix)) = pending.pop() {
      visited += 1;
      if visited > MAX_DIRECTORIES {
        return Err(CodecError::Message(format!(
          "ISO 9660 image contains more than {MAX_DIRECTORIES} directories"
        )));
      }

      for entry in directory.entries() {
        let entry = entry
          .map_err(|err| CodecError::Message(format!("Could not read a directory entry: {err}")))?;

        // Every directory carries entries for itself and for its parent.
        if entry.is_special() {
          continue;
        }

        let name = Self::entry_name(&entry)?;
        let path = format!("{prefix}{name}");

        if entry.is_directory() {
          let reference = entry
            .as_dir_ref(image)
            .map_err(|err| CodecError::Message(format!("Could not open '{path}': {err}")))?;

          pending.push((image.open_dir(reference), format!("{path}/")));
          continue;
        }

        let extent = entry.header().extent.read();
        let length = entry.header().data_len.read();

        if ignore_out_of_bounds
          && length > 0
          && u64::from(extent) * DESCRIPTOR_SIZE as u64 >= image_len as u64
        {
          debug!("Skipping out-of-bounds ISO 9660 entry '{path}'");
          continue;
        }

        let data = image
          .read_file(&entry)
          .map_err(|err| CodecError::Message(format!("Could not read '{path}': {err}")))?;

        entries.push(IsoFileEntry {
          name: path,
          data,
          extent,
        });
      }
    }

    Ok(entries)
  }

  /// The image's ISO 9660 directory tree, which its Primary Volume Descriptor
  /// holds.
  ///
  /// Matched on the Level 1 variant rather than an exact `EntryType`: Hadris
  /// flags the primary tree with `supports_rrip` when the image carries Rock
  /// Ridge, so an exact match would miss precisely the images that carry it.
  fn root_of(image: &IsoImage<Cursor<&[u8]>>) -> Result<hadris_iso::read::RootDir, CodecError> {
    image
      .root_dirs()
      .iter()
      .find(|root| matches!(root.entry_type(), EntryType::Level1 { .. }))
      .copied()
      .ok_or_else(|| {
        CodecError::Message("ISO 9660 image carries no primary directory tree to read".to_owned())
      })
  }

  /// The name a directory entry is known by inside the image.
  ///
  /// Taken from the record's own identifier rather than from `display_name`,
  /// which prefers a Rock Ridge alternate name and would answer `FOO.BIN` where
  /// the ISO 9660 namespace holds `FOO.BAR;1`. The `;1` belongs to the
  /// identifier but not to the name, and the writer supplies its own, so it is
  /// dropped here and comes back on the rebuild.
  fn entry_name(entry: &hadris_iso::read::DirEntry) -> Result<String, CodecError> {
    let raw = entry.record.name();
    let raw = match raw.iter().position(|byte| *byte == b';') {
      Some(index) => &raw[..index],
      None => raw,
    };

    let name = std::str::from_utf8(raw).map_err(|err| {
      CodecError::Message(format!(
        "ISO 9660 file identifier is not valid text and cannot be named: {err}"
      ))
    })?;

    if name.is_empty() {
      return Err(CodecError::Message(
        "ISO 9660 directory entry carries an empty file identifier".to_owned(),
      ));
    }

    Ok(name.to_owned())
  }

  /// Places the artifacts in the order the metadata gives.
  ///
  /// The two must agree exactly: an unnamed file has nowhere to go and a
  /// twice-named one is ambiguous.
  fn order<'a>(
    metadata: &IsoMetadata,
    files: &'a [DecodedArtifact],
  ) -> Result<Vec<&'a DecodedArtifact>, CodecError> {
    let mut remaining: Vec<&DecodedArtifact> = files.iter().collect();
    let mut ordered = Vec::with_capacity(remaining.len());
    let mut placed = BTreeSet::new();

    for path in &metadata.file_order {
      // Asked before looking, because a placed file is no longer among those a
      // later entry could match.
      if !placed.insert(path.clone()) {
        return Err(CodecError::Message(format!(
          "ISO metadata names '{path}' more than once"
        )));
      }

      let Some(index) = remaining.iter().position(|artifact| artifact.name == *path) else {
        return Err(CodecError::Message(format!(
          "ISO metadata names '{path}', which is not an artifact of this decode"
        )));
      };

      ordered.push(remaining.remove(index));
    }

    // An artifact the metadata does not account for has no recorded place to go.
    if let Some(unplaced) = remaining.first() {
      return Err(CodecError::Message(format!(
        "ISO metadata does not name '{}', which is an artifact of this decode",
        unplaced.name
      )));
    }

    Ok(ordered)
  }

  fn input_files(ordered: &[&DecodedArtifact]) -> Result<InputFiles, CodecError> {
    let mut roots = Vec::new();

    for artifact in ordered {
      let path = artifact.name.as_str();
      let mut components: Vec<&str> = path.split('/').collect();
      let leaf = components.pop().unwrap_or_default();

      Self::insert_file(&mut roots, path, &components, leaf, artifact.data.clone())?;
    }

    Ok(InputFiles {
      path_separator: PathSeparator::ForwardSlash,
      files: roots,
    })
  }

  fn insert_file(
    entries: &mut Vec<IsoFile>,
    path: &str,
    components: &[&str],
    leaf: &str,
    data: Vec<u8>,
  ) -> Result<(), CodecError> {
    let Some((directory, rest)) = components.split_first() else {
      Self::validate_file_name(leaf).map_err(|reason| {
        CodecError::Message(format!(
          "'{path}' cannot be rebuilt as plain ISO 9660 Level 1: {reason}"
        ))
      })?;

      entries.push(IsoFile::File {
        name: Arc::new(leaf.to_owned()),
        contents: data,
      });

      return Ok(());
    };

    let index = match entries.iter().position(
      |entry| matches!(entry, IsoFile::Directory { name, .. } if name.as_ref() == directory),
    ) {
      Some(index) => index,
      None => {
        Self::validate_directory_name(directory).map_err(|reason| {
          CodecError::Message(format!(
            "'{path}' cannot be rebuilt as plain ISO 9660 Level 1: {reason}"
          ))
        })?;

        entries.push(IsoFile::Directory {
          name: Arc::new((*directory).to_owned()),
          children: Vec::new(),
        });

        entries.len() - 1
      }
    };

    let IsoFile::Directory { children, .. } = &mut entries[index] else {
      unreachable!("the position was found on a directory")
    };

    Self::insert_file(children, path, rest, leaf, data)
  }

  fn write_options(volume_identifier: &str) -> IsoFormatOptions {
    IsoFormatOptions {
      volume_name: volume_identifier.to_owned(),
      system_id: None,
      volume_set_id: None,
      publisher_id: None,
      preparer_id: None,
      application_id: None,
      sector_size: DESCRIPTOR_SIZE,
      path_separator: PathSeparator::ForwardSlash,
      features: CreationFeatures {
        filenames: BaseIsoLevel::Level1 {
          supports_lowercase: false,
          supports_rrip: false,
        },
        long_filenames: false,
        joliet: None,
        rock_ridge: None,
        el_torito: None,
        hybrid_boot: None,
      },
      strict_charset: false,
    }
  }
}
