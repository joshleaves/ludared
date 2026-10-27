use log::*;
use serde::Deserialize;
use std::collections::BTreeSet;

use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use metadata::BLOCK_SIZE;
use metadata::IsoMetadata;
use metadata::METADATA_NAME;
use volume::DESCRIPTOR_SECTOR;
use volume::DESCRIPTOR_TYPE_PRIMARY;
use volume::STANDARD_IDENTIFIER;

pub(crate) mod metadata;
mod rebuild;
pub(crate) mod volume;

#[cfg(test)]
mod tests;

const CODEC_ID: &str = "std/cdrom/iso9660";
const CODEC_NAME: &str = "ISO 9660 CD-ROM image";
const CODEC_DESC: &str = r#"ISO 9660 CD-ROM image codec

Extracts the files of an ISO 9660 image, preserving its directory hierarchy,
alongside an ISO9660.json artifact describing the image's layout: the Volume
Identifier it carries, where each of its structures sits, and where every file
lies on the disc.

Reassembling follows that layout rather than laying the files out again. Every
file goes back in the blocks it held, every directory keeps the records it held
and the bytes they filled, and the path tables, the volume descriptor and the
System Area are written from what the image declared. An unchanged decode and
rebuild therefore reproduces the image byte for byte, which matters for a disc
whose game seeks to blocks of its own rather than through the filesystem.

Images are read and written as plain ISO 9660 in 2048-byte logical blocks. The
codec works on the logical image; converting to and from the 2352-byte sectors
of a raw disc image is the Mode 1/2352 codec's business.

What the layout records

The metadata is never a file of the image, and is always the first artifact a
decode produces, which is where a rebuild looks for it.

Its file_order must name every file artifact exactly once, and the layout must
describe every file the image holds: a file added to or removed from the image
has to be added to or removed from both, and a mismatch between the two is
refused rather than guessed at. A file added to file_order alone is written into
the directory its path names, if that directory has room.

Changing a file

A file of the same size goes back where it was, and a smaller one keeps its
blocks, which the layout saves it from having to find again: the space it no
longer fills is left empty, since re-laying out an image is what a file that grew
asks for, not what one that gave room back does.

What happens to a file that no longer fits the blocks it held is what the
placement argument decides:

  * shift (what a decode added now is recorded with) keeps every region in the
    order the image had it: the file grows where it was, and every region after it
    moves along by exactly the blocks it needs and no more. No file changes
    position relative to its neighbours, and the image is lengthened only when the
    last region runs past its end. The cost is that every region after a grown
    file moves, and a region whose position something outside the filesystem
    states — a boot record's own block numbers, say — moves with it whether or not
    that something still describes it.
  * relocate is what the codec did before it had policies. A file keeps its
    extent while it fits, is extended in place when the blocks that follow are
    free, and otherwise moves to the first free room in the image, or past the
    image's end. Moving a file changes its block, which a game that seeks to that
    block would notice, so a move is logged; a move that would leave a structure
    this codec does not rewrite describing where the file used to be is refused.
  * preserve moves nothing: a change is written when it fits the blocks the file
    holds or the free blocks that follow them, and refused otherwise rather than
    answered by moving a file or shifting one. An image is lengthened only for an
    image that holds nothing beyond its end, since a longer image moves whatever
    lies past it.

An image past the blocks it holds is grown when a file under `relocate` or
`shift` needs the room. A mixed-mode disc keeps its last file on the track that
follows, and growing the data track moves that track along: the records that
describe data beyond the image move with the blocks the image grew by, keeping
their distance from its end, and so does the Volume Space Size where it declared
blocks past that end. A file whose extent straddles the image's end is refused
instead: one run of blocks across the boundary cannot be kept whole by a longer
image.

Under `shift` every reference the codec knows how to rewrite follows the regions
it moves: each file's record, each directory's own record and the record its
parent holds for it, the path table entry that names it, the blocks the descriptor
names for the path tables, and the Volume Space Size. What it cannot follow is
anything outside the filesystem that states a block number of its own — a boot
record's pointers, a program that seeks to a block it was built with — which keeps
its number while the blocks under it move.

The System Area, the areas a volume descriptor reserves, padding, and any data
lying outside the filesystem are preserved exactly as they were, and a rebuild
that would write over them is refused.

Arguments (JSON):
  out_of_bounds_entries (default: "preserve")
    How to handle directory records pointing at blocks the image does not hold,
    as a mixed-mode disc does for files kept on a later track.

    preserve   Keep the records as the image holds them, and their contents where
               they lie. Nothing carries those contents, so their records are
               written back unchanged (default)
    error      Reject the image
    ignore     Drop those entries. They are not preserved when rebuilding the
               image

  placement (default: "relocate" where args name none)
    How a file that no longer fits the blocks it held is given room.

    relocate   Keep a file's extent where it fits, grow it into the free blocks
               that follow, and otherwise move it to free room elsewhere
    shift      Reconstruct the image in its original order, shifting the regions
               that follow a file that grew and updating every reference to them
    preserve   Keep every extent exactly where it is, and refuse a change that
               would need a file moved or shifted

    Args that name no policy keep the planner the codec planned with before it
    had policies, which is "relocate": a manifest written earlier rebuilds the
    way it did. A decode added now is recorded with {"placement":"shift"} in the
    manifest, like args the caller named, so a decode keeps the policy it was
    added under rather than changing under it.

Example:
  {"placement":"shift","out_of_bounds_entries":"error"}

Not supported, and refused rather than written wrongly:

  * logical block sizes other than 2048 bytes
  * files written in several extents, interleaved files, and files with an
    extended attribute record, for any file whose contents change
  * volume descriptors besides the primary one, and second copies of the path
    tables, for any change that would move a file or resize a directory: those
    structures describe the same tree and this codec does not rewrite them. A
    second copy of the path tables also refuses a shift that would move a
    directory or a path table, which is where a directory's own block is stated
    besides its records
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
      .and_then(|sector| sector.checked_mul(BLOCK_SIZE))
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

  /// Reads the image's files and the layout they sit in.
  ///
  /// The files lead the artifacts in the order they sit on the disc, after the
  /// metadata describing that order and the rest of the layout.
  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding ISO 9660 image");

    let args = Iso9660CodecArgs::from_args(args)?;
    let reading = volume::read(data, args.out_of_bounds_entries)?;

    let metadata = IsoMetadata {
      volume_identifier: reading.volume_identifier,
      file_order: reading.files.iter().map(|file| file.name.clone()).collect(),
      layout: reading.layout,
    };

    // The metadata leads, so the manifest records it first and a rebuild finds
    // it by position.
    let mut artifacts = Vec::with_capacity(reading.files.len() + 1);

    artifacts.push(DecodedArtifact {
      name: METADATA_NAME.to_owned(),
      data: metadata.to_json()?,
    });

    artifacts.extend(reading.files.into_iter().map(|file| DecodedArtifact {
      name: file.name,
      data: file.data,
    }));

    Ok(artifacts)
  }

  /// Rebuilds the image the metadata describes, with the files it was given.
  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"encode"))
  )]
  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding ISO 9660 image");

    let Iso9660CodecArgs {
      placement,
      out_of_bounds_entries: _,
    } = Iso9660CodecArgs::from_args(args)?;

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

    rebuild::build(&metadata, &ordered, placement)
  }
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutOfBoundsEntries {
  Error,
  Ignore,
  #[default]
  Preserve,
}

/// How a rebuild that has to find a file more room goes about it.
///
/// The three are the same planner with a different idea of what an extent is
/// worth: `shift` treats the image as one sequence it re-lays out, `relocate`
/// moves the one file that no longer fits, and `preserve` will not move
/// anything at all.
#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Placement {
  /// Reconstruct the image in its original order: a file that grows past the
  /// blocks it had keeps its extent and shifts what follows it along, so that
  /// every region keeps its place in the sequence. Nothing is ever relocated.
  Shift,

  /// Keep a file at its extent where it fits, grow it into the free room that
  /// follows, and otherwise move it to free room elsewhere in the image (or
  /// lengthen the image), leaving every other region where it was. A moved file
  /// is logged, since a game that seeks to a block of its own would notice.
  ///
  /// This is what the codec did before it had placement policies, and it is what
  /// a decode whose args name no policy still gets.
  #[default]
  Relocate,

  /// Keep every existing extent exactly where it is: a change is allowed when
  /// it fits the blocks the file already holds or the free room that follows
  /// them, and refused otherwise, rather than moving a file or shifting one.
  /// The image is lengthened only for an image that holds nothing beyond its
  /// end, since a longer image moves the blocks that follow it.
  Preserve,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Iso9660CodecArgs {
  #[serde(default)]
  out_of_bounds_entries: OutOfBoundsEntries,

  #[serde(default)]
  placement: Placement,
}

impl LudaredCodecArgs for Iso9660CodecArgs {}

pub(crate) struct Iso9660;

impl Iso9660 {
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
}
