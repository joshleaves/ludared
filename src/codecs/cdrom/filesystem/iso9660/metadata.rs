//! What a rebuild needs to know about an ISO 9660 image.
//!
//! An ISO 9660 image is a filesystem laid out in logical blocks: a System Area,
//! a volume descriptor set, path tables, a tree of directories, and the
//! contents of the files those directories list. Rebuilding that tree with a
//! generic ISO writer lays the whole thing out again, which moves every file and
//! leaves the System Area, the padding and anything the filesystem does not
//! describe behind.
//!
//! The metadata below is that layout, written down: where each structure and
//! each file sits, down to the byte. A rebuild follows it rather than inventing
//! a layout of its own, so an unchanged decode and encode reproduces the image
//! byte for byte.
//!
//! Bytes the codec does not interpret — the System Area, the areas a volume
//! descriptor reserves, padding, and data lying outside the filesystem — are
//! recorded as [`Opaque`] runs and written back where they were. Nothing is
//! discarded for being unrecognized.

use serde::Deserialize;
use serde::Serialize;

pub(super) use crate::codecs::cdrom::common::BLOCK_SIZE;
pub(super) use crate::codecs::cdrom::common::Hex;
use crate::codecs::errors::CodecError;

/// The name the metadata artifact is given.
pub(crate) const METADATA_NAME: &str = "ISO9660.json";

/// The metadata of an ISO 9660 image, as a rebuild needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IsoMetadata {
  /// The image's Volume Identifier, without the padding ISO 9660 mandates.
  pub volume_identifier: String,

  /// The image's files, in the order they sit on the disc.
  ///
  /// It must name every file artifact exactly once: a file added to or removed
  /// from the image has to be added to or removed from both the layout and the
  /// manifest's outputs, and a mismatch between the two is refused rather than
  /// guessed at.
  pub file_order: Vec<String>,

  /// Where everything the image is made of sits.
  pub layout: Layout,
}

impl IsoMetadata {
  pub(crate) fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the ISO metadata: {err}")))
  }

  pub(crate) fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the ISO metadata artifact: {err}"))
    })
  }
}

/// The layout of an ISO 9660 image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Layout {
  /// How many logical blocks the image itself spans.
  ///
  /// The Volume Space Size a volume descriptor declares is the filesystem's own
  /// metadata and may exceed this: a disc whose last file lives on a track the
  /// image does not hold declares blocks this image does not carry.
  pub image_sectors: u32,

  /// The Volume Space Size the Primary Volume Descriptor declares.
  pub volume_space_size: u32,

  /// The System Area: the blocks ISO 9660 reserves before the volume descriptor
  /// set, which a platform keeps its own boot data in.
  #[serde(default, skip_serializing_if = "Hex::is_empty")]
  pub system_area: Hex,

  /// Runs of bytes the codec does not interpret, in the order they sit in the
  /// image.
  ///
  /// What lands here is everything the filesystem does not describe: a
  /// supplementary or boot descriptor, the bytes a descriptor reserves beyond
  /// the fields it defines, padding left by a mastering tool, and data lying
  /// outside the filesystem. A rebuild writes these back where they were, and
  /// refuses any operation that would write over them.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub opaque: Vec<Opaque>,

  /// The fields of the image's Primary Volume Descriptor.
  pub primary: Primary,

  /// The image's path tables.
  pub path_table: PathTable,

  /// The image's directories, the root first.
  pub directories: Vec<Directory>,

  /// Whether the image carries a volume descriptor other than its primary
  /// descriptor: a supplementary descriptor, say, which describes a second tree
  /// over the same files.
  ///
  /// Those trees are preserved exactly as they stand but a rebuild does not
  /// rewrite them, so a rebuild that would move a file or resize a directory is
  /// refused while one is present rather than leave it describing where its
  /// files used to be.
  #[serde(default, skip_serializing_if = "is_false")]
  pub secondary_descriptors: bool,
}

impl Layout {
  /// The directory the filesystem is rooted at.
  pub(crate) fn root(&self) -> Option<&Directory> {
    self.directories.iter().find(|dir| dir.path.is_empty())
  }

  /// Every run of bytes the codec does not interpret, with its bytes.
  pub(crate) fn opaque_runs(&self) -> Vec<(u64, Vec<u8>)> {
    self
      .opaque
      .iter()
      .map(|run| (run.offset, run.data.as_bytes().to_vec()))
      .collect()
  }
}

/// A run of bytes the codec does not interpret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Opaque {
  /// Where the run starts, as a byte offset into the image.
  pub offset: u64,

  /// The run's bytes.
  pub data: Hex,
}

/// The fields of a Primary Volume Descriptor.
///
/// A field is recorded as it is read with the padding ISO 9660 mandates
/// trimmed, and written back padded the same way. A field padded with anything
/// else is refused rather than re-encoded into what a reader would take for the
/// same value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Primary {
  /// The System Identifier, which a platform may use for its own purposes.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub system_identifier: String,

  /// The Volume Set Identifier.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub volume_set_identifier: String,

  /// The Publisher Identifier.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub publisher_identifier: String,

  /// The Preparer Identifier: who mastered the volume.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub preparer_identifier: String,

  /// The Application Identifier.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub application_identifier: String,

  /// The identifier of the file holding the volume's copyright notice.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub copyright_file_identifier: String,

  /// The identifier of the file holding an abstract of the volume.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub abstract_file_identifier: String,

  /// The identifier of the file holding a bibliographic description.
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub bibliographic_file_identifier: String,

  /// The descriptor's Application Use area: the bytes ISO 9660 reserves for
  /// whatever a writer wants to keep beside the volume and does not interpret.
  ///
  /// Recorded as it stands, with the zeroes a writer padded its end with
  /// trimmed, and written back padded with zeroes: the same bytes either way,
  /// where spelling out an area a writer left empty would say nothing in 512
  /// bytes. A rebuild of a descriptor that lost them would leave an image whose
  /// master data no longer says what it said.
  #[serde(default, skip_serializing_if = "Hex::is_empty")]
  pub application_use: Hex,

  /// The volume's creation date and time.
  ///
  /// Seventeen bytes: the sixteen digits ISO 9660 writes dates as, and the byte
  /// after them, which counts fifteen-minute steps from GMT. Recorded as it
  /// stands because writers disagree on that last byte and on whether a zeroed
  /// field means "no date" or "the start of the epoch".
  #[serde(default, skip_serializing_if = "Hex::is_empty")]
  pub volume_creation_date: Hex,

  /// The volume's modification date and time.
  #[serde(default, skip_serializing_if = "Hex::is_empty")]
  pub volume_modification_date: Hex,

  /// The date and time after which the volume may be ignored.
  #[serde(default, skip_serializing_if = "Hex::is_empty")]
  pub volume_expiration_date: Hex,

  /// The date and time from which the volume is effective.
  #[serde(default, skip_serializing_if = "Hex::is_empty")]
  pub volume_effective_date: Hex,

  /// How many volumes the volume set holds.
  #[serde(default = "one", skip_serializing_if = "is_one")]
  pub volume_set_size: u16,

  /// This volume's number within its volume set.
  #[serde(default = "one", skip_serializing_if = "is_one")]
  pub volume_sequence_number: u16,

  /// The size of the image's logical blocks, in bytes.
  #[serde(default = "block_size", skip_serializing_if = "is_block_size")]
  pub logical_block_size: u16,

  /// The descriptor's File Structure Version: the version of the
  /// specification of directory records and path tables the image declares.
  ///
  /// ECMA-119 words the field as naming a structure version and prescribes 1
  /// for a primary descriptor, so an image carrying anything else states its
  /// structure differently than this codec reads it — but the field changes
  /// no other byte's meaning, and this codec's concern is writing the image
  /// back as it stood. The value is therefore recorded and written verbatim
  /// rather than validated, and images declaring the usual 1 carry it in the
  /// default, so their metadata reads the same as it always has.
  #[serde(default = "one_version", skip_serializing_if = "is_one_version")]
  pub file_structure_version: u8,
}

/// The image's path tables: the directory tree flattened into the order a
/// reader walks it.
///
/// ISO 9660 keeps two copies of the same table, one with its numbers written
/// little-endian and one big-endian, so that either kind of machine reads one
/// without byte-swapping. They are the same tree twice, so the codec holds the
/// tree once and writes both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PathTable {
  /// The block the little-endian table starts at.
  pub l_sector: u32,

  /// The block the big-endian table starts at.
  pub m_sector: u32,

  /// The block holding the optional second little-endian table, if any.
  ///
  /// An image carrying one declares a copy the codec does not maintain, so a
  /// rebuild that would move anything is refused rather than leave that copy
  /// pointing where its directories no longer are.
  #[serde(default, skip_serializing_if = "is_zero")]
  pub optional_l_sector: u32,

  /// The block holding the optional second big-endian table, if any.
  #[serde(default, skip_serializing_if = "is_zero")]
  pub optional_m_sector: u32,

  /// The table's entries, in the order the table lists them.
  pub entries: Vec<PathTableEntry>,
}

impl PathTable {
  /// How many bytes the table's entries take, which is the Path Table Size the
  /// descriptor declares.
  pub(crate) fn size(&self) -> usize {
    self.entries.iter().map(PathTableEntry::size).sum()
  }

  /// Whether the image carries a second copy of its path tables.
  pub(crate) fn has_copy(&self) -> bool {
    self.optional_l_sector != 0 || self.optional_m_sector != 0
  }
}

/// One directory's place in a path table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PathTableEntry {
  /// The directory's identifier, the root's being a single zero byte.
  pub name: String,

  /// The block the directory's own extent starts at.
  pub extent: u32,

  /// The number of the entry describing the directory's parent, counting from
  /// one.
  pub parent: u16,
}

impl PathTableEntry {
  /// How many bytes this entry takes: a fixed part, the identifier, and the pad
  /// byte an entry of odd length takes to come out even.
  pub(crate) fn size(&self) -> usize {
    8 + self.name.len() + usize::from(!self.name.len().is_multiple_of(2))
  }
}

/// One directory of the image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Directory {
  /// The path the directory's files are extracted under, the root's being empty.
  pub path: String,

  /// The block the directory's extent starts at.
  pub extent: u32,

  /// The length of the directory's data, in bytes.
  pub length: u32,

  /// The directory's records, in the order the extent lists them, the entries
  /// for the directory itself and its parent included.
  pub records: Vec<Record>,
}

impl Directory {
  /// The record the directory carries for itself, which every ISO 9660
  /// directory leads with.
  pub(crate) fn own_record(&self) -> Option<&Record> {
    self.records.first().filter(|record| record.name == "\u{0}")
  }
}

/// One directory record: what a directory says about a file or a directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
  /// The record's file identifier, as ISO 9660 writes it.
  ///
  /// A file's identifier carries a version suffix, `SAKURA.BIN;1`, which the
  /// extracted artifact's name does not. A directory's identifier takes none.
  pub name: String,

  /// The block the record's data starts at.
  pub extent: u32,

  /// The length of the record's data, in bytes.
  pub length: u32,

  /// The record's flags, which ISO 9660 defines the bits of.
  #[serde(default, skip_serializing_if = "is_zero_u8")]
  pub flags: u8,

  /// When the record was written, as the seven bytes ISO 9660 records dates in.
  pub date: Hex,

  /// This volume's number within its volume set.
  #[serde(default = "one", skip_serializing_if = "is_one")]
  pub volume_sequence_number: u16,

  /// The record's system use area, which extensions such as Rock Ridge keep
  /// their own data in.
  ///
  /// Its bytes are preserved as they stand, but a rebuild will not move a file
  /// whose record carries one: an extension is free to record a file's position
  /// there, and moving the file would leave that record describing where the
  /// file used to be.
  #[serde(default, skip_serializing_if = "Hex::is_empty")]
  pub system_use: Hex,

  /// The bytes the directory holds between the record before this one and this
  /// one.
  ///
  /// ISO 9660 does not let a record straddle a block boundary, and the writers
  /// disagree on the room a record does not fill: one moves the record that
  /// would end exactly at the boundary on to the next block, another lets it
  /// stay. The bytes that were there say which, and a rebuild writes them back.
  #[serde(default, skip_serializing_if = "Hex::is_empty")]
  pub pad: Hex,
}

impl Record {
  /// The name the record's artifact is given.
  pub(crate) fn artifact_name(&self) -> &str {
    artifact_name(&self.name)
  }

  /// Whether the record describes a directory.
  pub(crate) fn is_directory(&self) -> bool {
    self.flags & FLAG_DIRECTORY != 0
  }

  /// Whether the record describes a file rather than a directory.
  pub(crate) fn is_file(&self) -> bool {
    !self.is_directory()
  }

  /// How many bytes of the record's data lie inside an image of `image_len`
  /// bytes.
  ///
  /// A record may point at blocks the image does not hold: a mixed-mode disc may
  /// keep a file on the track that follows the one the image carries. Nothing
  /// carries such a file's contents, so its record is preserved as it stands.
  pub(crate) fn in_image(&self, image_len: usize) -> usize {
    let start = u64::from(self.extent) * BLOCK_SIZE as u64;

    usize::try_from(start)
      .ok()
      .and_then(|start| image_len.checked_sub(start))
      .map_or(0, |available| available.min(self.length as usize))
  }

  /// How many bytes the record takes: the fixed part, the identifier, the pad
  /// byte an identifier of even length takes, and the system use area.
  pub(crate) fn size(&self) -> usize {
    RECORD_HEADER
      + self.name.len()
      + usize::from(self.name.len().is_multiple_of(2))
      + self.system_use.len()
  }

  /// What the record describes that this codec cannot rebuild, if anything.
  pub(crate) fn unsupported(&self) -> Option<&'static str> {
    if self.flags & FLAG_MULTI_EXTENT != 0 {
      return Some("a file written in several extents");
    }

    if self.flags & FLAG_ASSOCIATED != 0 {
      return Some("an associated file, which shares the extent of another");
    }

    None
  }
}

/// The name a record's artifact is given: its identifier with the version
/// suffix ISO 9660 writes and a writer supplies again taken off.
pub(crate) fn artifact_name(identifier: &str) -> &str {
  identifier
    .split_once(';')
    .map_or(identifier, |(name, _)| name)
}

/// The bit of a record's flags marking a directory.
pub(crate) const FLAG_DIRECTORY: u8 = 0x02;

/// The bit of a record's flags marking a file written in several extents.
pub(crate) const FLAG_MULTI_EXTENT: u8 = 0x80;

/// The bit of a record's flags marking an associated file.
pub(crate) const FLAG_ASSOCIATED: u8 = 0x04;

/// How much of a directory record precedes its file identifier.
pub(crate) const RECORD_HEADER: usize = 33;

fn one() -> u16 {
  1
}

fn is_one(value: &u16) -> bool {
  *value == 1
}

fn one_version() -> u8 {
  1
}

fn is_one_version(value: &u8) -> bool {
  *value == 1
}

fn block_size() -> u16 {
  BLOCK_SIZE as u16
}

fn is_block_size(value: &u16) -> bool {
  *value == block_size()
}

fn is_zero(value: &u32) -> bool {
  *value == 0
}

fn is_zero_u8(value: &u8) -> bool {
  *value == 0
}

fn is_false(value: &bool) -> bool {
  !*value
}
