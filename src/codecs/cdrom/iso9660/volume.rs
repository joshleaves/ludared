//! Reading an ISO 9660 image into its layout, and writing its structures back.
//!
//! Everything here works in logical blocks: the codec never sees the sector
//! headers, error correction or the raw 2352-byte sectors a disc image may hold
//! them in. That is the Mode 1/2352 codec's business.
use log::*;
use std::collections::BTreeSet;
use std::collections::VecDeque;

use super::OutOfBoundsEntries;
use super::metadata::BLOCK_SIZE;
use super::metadata::Directory;
use super::metadata::Hex;
use super::metadata::Layout;
use super::metadata::Opaque;
use super::metadata::PathTable;
use super::metadata::PathTableEntry;
use super::metadata::Primary;
use super::metadata::RECORD_HEADER;
use super::metadata::Record;
use crate::codecs::cdrom::common_disk::ByteRange;
use crate::codecs::cdrom::common_disk::block_offset;
use crate::codecs::errors::CodecError;

/// The logical block the volume descriptor set starts at.
pub(crate) const DESCRIPTOR_SECTOR: u32 = 16;

/// How many logical blocks the System Area spans.
pub(crate) const SYSTEM_AREA_SECTORS: u32 = 16;

/// The size of the System Area, in bytes.
pub(crate) const SYSTEM_AREA_SIZE: usize = SYSTEM_AREA_SECTORS as usize * BLOCK_SIZE;

/// The descriptor type opening a volume descriptor set.
pub(crate) const DESCRIPTOR_TYPE_PRIMARY: u8 = 0x01;

/// The descriptor type ending a volume descriptor set.
pub(crate) const DESCRIPTOR_TYPE_TERMINATOR: u8 = 0xFF;

/// The identifier every ISO 9660 volume descriptor carries.
pub(crate) const STANDARD_IDENTIFIER: &[u8; 5] = b"CD001";

/// How many descriptors a volume descriptor set may hold before it is taken to
/// be malformed.
const MAX_DESCRIPTORS: usize = 64;

/// How many directories a walk will open before the tree is taken to be looping.
const MAX_DIRECTORIES: usize = 4096;

/// The width of the date a volume descriptor holds, in bytes.
const PRIMARY_DATE: usize = 17;

/// Where a Primary Volume Descriptor's Application Use area starts, and how
/// many bytes ISO 9660 reserves for it.
const PRIMARY_APPLICATION_USE: usize = 883;
const PRIMARY_APPLICATION_USE_LEN: usize = 512;

/// The byte of a descriptor holding its File Structure Version.
const PRIMARY_FILE_STRUCTURE_VERSION: usize = 881;

/// Where each field a rebuild models sits in a Primary Volume Descriptor, and
/// what to call it when its value cannot be written back.
pub(crate) const PRIMARY_FIELDS: &[(&str, usize, usize)] = &[
  ("a descriptor header", 0, 7),
  ("a System Identifier", 8, 32),
  ("a Volume Identifier", 40, 32),
  ("a Volume Space Size", 80, 8),
  ("a Volume Set Size", 120, 4),
  ("a Volume Sequence Number", 124, 4),
  ("a Logical Block Size", 128, 4),
  ("a Path Table Size", 132, 8),
  ("path table locations", 140, 16),
  ("a root directory record", 156, 34),
  ("a Volume Set Identifier", 190, 128),
  ("a Publisher Identifier", 318, 128),
  ("a Preparer Identifier", 446, 128),
  ("an Application Identifier", 574, 128),
  ("a Copyright File Identifier", 702, 37),
  ("an Abstract File Identifier", 739, 37),
  ("a Bibliographic File Identifier", 776, 37),
  ("volume dates", 813, 68),
  ("a File Structure Version", 881, 1),
  (
    "an Application Use area",
    PRIMARY_APPLICATION_USE,
    PRIMARY_APPLICATION_USE_LEN,
  ),
];

/// An ISO 9660 image read into the layout a rebuild follows, and its files.
pub(crate) struct Reading {
  pub volume_identifier: String,
  pub layout: Layout,
  pub files: Vec<FileEntry>,
}

/// A file the image holds, under the name its artifact is given.
pub(crate) struct FileEntry {
  /// The path the file sits at in the image, with no version suffix.
  pub name: String,

  /// The block the file's data starts at.
  pub extent: u32,

  /// How many file bytes the image actually holds.
  pub length: usize,

  /// The file's contents, or as much of them as the image holds.
  pub data: Vec<u8>,
}

/// Reads an image into the layout a rebuild follows.
pub(crate) fn read(image: &[u8], out_of_bounds: OutOfBoundsEntries) -> Result<Reading, CodecError> {
  read_with_contents(image, out_of_bounds, true)
}

/// Reads the filesystem layout without copying file contents out of the image.
pub(crate) fn read_layout(
  image: &[u8],
  out_of_bounds: OutOfBoundsEntries,
) -> Result<Reading, CodecError> {
  read_with_contents(image, out_of_bounds, false)
}

fn read_with_contents(
  image: &[u8],
  out_of_bounds: OutOfBoundsEntries,
  read_file_contents: bool,
) -> Result<Reading, CodecError> {
  let image_len = image.len();

  if image_len == 0 || !image_len.is_multiple_of(BLOCK_SIZE) {
    return Err(CodecError::Message(format!(
      "An ISO 9660 image is a whole number of {BLOCK_SIZE}-byte logical blocks, and this one is \
       {image_len} bytes"
    )));
  }

  let image_sectors = u32::try_from(image_len / BLOCK_SIZE).map_err(|_| {
    CodecError::Message("The image holds more logical blocks than ISO 9660 can address".to_owned())
  })?;

  let descriptor = block(image, DESCRIPTOR_SECTOR)?;

  if descriptor[0] != DESCRIPTOR_TYPE_PRIMARY || &descriptor[1..6] != STANDARD_IDENTIFIER {
    return Err(CodecError::Message(
      "ISO 9660 primary volume descriptor is missing or malformed".to_owned(),
    ));
  }

  let primary = parse_primary(descriptor)?;
  let volume_identifier = read_text(descriptor, 40, 32, "Volume Identifier")?;
  let volume_space_size = read_both_ended(descriptor, 80, 4, "Volume Space Size")?;
  let secondary_descriptors = descriptor_set_holds_more(image)?;

  let mut coverage = Coverage::new(image_len);

  coverage.cover(0, SYSTEM_AREA_SIZE, "the System Area".to_owned())?;

  // The descriptor set is covered field by field rather than block by block:
  // the bytes a rebuild writes into it are the ones it models, and the bytes in
  // between are preserved as the bytes they are.
  for (what, offset, width) in PRIMARY_FIELDS {
    coverage.cover(
      DESCRIPTOR_SECTOR as usize * BLOCK_SIZE + offset,
      *width,
      format!("the descriptor's {what}"),
    )?;
  }

  let path_table = parse_path_tables(image, descriptor, &mut coverage)?;

  let root = parse_record(
    &descriptor[156..190],
    "the root directory record in the descriptor",
  )?;

  let (directories, mut files) = walk(
    image,
    &root,
    out_of_bounds,
    read_file_contents,
    &mut coverage,
  )?;

  let root_directory = directories
    .iter()
    .find(|directory| directory.path.is_empty())
    .ok_or_else(|| CodecError::Message("The image's tree holds no root directory".to_owned()))?;

  // The descriptor's copy of the root record and the directory's own entry
  // describe one directory, and a rebuild writes both from the directory.
  let own = root_directory.own_record().ok_or_else(|| {
    CodecError::Message("The image's root directory holds no record for itself".to_owned())
  })?;

  let mut derived = own.clone();
  derived.system_use = Hex::default();

  if serialize_record(&derived)? != serialize_record(&root)? {
    return Err(CodecError::Message(
      "The image's descriptor and its root directory disagree on the root record, which this codec \
       cannot rebuild"
        .to_owned(),
    ));
  }

  files.sort_by(|left, right| {
    left
      .extent
      .cmp(&right.extent)
      .then_with(|| left.name.cmp(&right.name))
  });

  // Two records resolving to one artifact path cannot both be rebuilt: the
  // name is the file's whole identity once the image is a set of artifacts.
  // Looked for by name rather than by extent, which the sort above orders by.
  let mut paths = BTreeSet::new();

  for file in &files {
    if !paths.insert(file.name.as_str()) {
      return Err(CodecError::Message(format!(
        "ISO 9660 image holds two files under the path '{}'",
        file.name
      )));
    }
  }

  if let Some((left, right)) = coverage.overlap() {
    return Err(CodecError::Message(format!(
      "The image holds {left} and {right} in the same bytes, which this codec cannot rebuild"
    )));
  }

  let system_area = &image[..SYSTEM_AREA_SIZE];

  let layout = Layout {
    image_sectors,
    volume_space_size,
    system_area: if system_area.iter().all(|byte| *byte == 0) {
      Hex::default()
    } else {
      Hex::from(system_area)
    },
    opaque: coverage.opaque_runs(image),
    primary,
    path_table,
    directories,
    secondary_descriptors,
  };

  verify_primary(descriptor, &volume_identifier, &layout)?;
  verify_path_table(image, &layout.path_table)?;

  Ok(Reading {
    volume_identifier,
    layout,
    files,
  })
}

/// The bytes of the logical block `sector` holds.
pub(crate) fn block(image: &[u8], sector: u32) -> Result<&[u8], CodecError> {
  let start = block_offset(u64::from(sector))
    .and_then(|offset| usize::try_from(offset).ok())
    .ok_or_else(|| CodecError::Message(format!("No logical block {sector} fits in the image")))?;

  image.get(start..start + BLOCK_SIZE).ok_or_else(|| {
    CodecError::Message(format!(
      "The image holds no logical block {sector}: it is shorter than its own descriptor set"
    ))
  })
}

/// Whether the descriptor set holds anything besides the primary descriptor and
/// the terminator that ends it.
fn descriptor_set_holds_more(image: &[u8]) -> Result<bool, CodecError> {
  let mut sector = DESCRIPTOR_SECTOR;
  let mut more = false;

  loop {
    if sector - DESCRIPTOR_SECTOR > MAX_DESCRIPTORS as u32 {
      return Err(CodecError::Message(format!(
        "ISO 9660 volume descriptor set is not terminated after {MAX_DESCRIPTORS} descriptors"
      )));
    }

    let descriptor = block(image, sector)?;

    if &descriptor[1..6] != STANDARD_IDENTIFIER {
      return Err(CodecError::Message(format!(
        "Logical block {sector} does not hold an ISO 9660 volume descriptor"
      )));
    }

    if descriptor[6] != 1 {
      return Err(CodecError::Message(format!(
        "The volume descriptor at block {sector} has version {}, and this codec writes version 1",
        descriptor[6]
      )));
    }

    if descriptor[0] == DESCRIPTOR_TYPE_TERMINATOR {
      return Ok(more);
    }

    // Everything the set holds besides the primary descriptor that leads it
    // and the terminator that ends it describes the same volume in a way a
    // rebuild does not rewrite: a supplementary or partition descriptor's
    // tree, a boot record's pointers, or a second Primary Volume Descriptor's
    // own root record, sizes and path table locations.
    if sector != DESCRIPTOR_SECTOR {
      more = true;
    }

    sector += 1;
  }
}

/// The bytes of a Primary Volume Descriptor, as a rebuild writes it.
///
/// `volume_space_size` is the size to write for the descriptor's Volume Space
/// Size: how many logical blocks the volume spans. It is not always how many
/// blocks the image holds — a disc may declare blocks past the end of its data
/// track — so the caller decides it, in [`crate::codecs::cdrom::iso9660::rebuild`]
/// for a rebuild and from the image itself when checking one.
pub(crate) fn serialize_primary(
  layout: &Layout,
  volume_identifier: &str,
  root: &Record,
  volume_space_size: u32,
) -> Result<Vec<u8>, CodecError> {
  let primary = &layout.primary;

  if primary.logical_block_size as usize != BLOCK_SIZE {
    return Err(CodecError::Message(format!(
      "The image's logical block size is {} bytes, and this codec works in {BLOCK_SIZE}-byte \
       blocks",
      primary.logical_block_size
    )));
  }

  let table = u32::try_from(layout.path_table.size()).map_err(|_| {
    CodecError::Message("The path tables hold more entries than ISO 9660 can describe".to_owned())
  })?;

  let mut bytes = vec![0u8; BLOCK_SIZE];

  bytes[0] = DESCRIPTOR_TYPE_PRIMARY;
  bytes[1..6].copy_from_slice(STANDARD_IDENTIFIER);
  bytes[6] = 1;

  put_text(&mut bytes, 8, 32, &primary.system_identifier)?;
  put_text(&mut bytes, 40, 32, volume_identifier)?;

  put_both_ended(&mut bytes, 80, 4, volume_space_size)?;
  put_both_ended(&mut bytes, 120, 2, u32::from(primary.volume_set_size))?;
  put_both_ended(
    &mut bytes,
    124,
    2,
    u32::from(primary.volume_sequence_number),
  )?;
  put_both_ended(&mut bytes, 128, 2, u32::from(primary.logical_block_size))?;
  put_both_ended(&mut bytes, 132, 4, table)?;

  // The path table locations are the one place ISO 9660 writes numbers of
  // different widths without a second copy: the little-endian table's address
  // is little-endian, and the big-endian table's address is big-endian.
  bytes[140..144].copy_from_slice(&layout.path_table.l_sector.to_le_bytes());
  bytes[144..148].copy_from_slice(&layout.path_table.optional_l_sector.to_le_bytes());
  bytes[148..152].copy_from_slice(&layout.path_table.m_sector.to_be_bytes());
  bytes[152..156].copy_from_slice(&layout.path_table.optional_m_sector.to_be_bytes());

  let root = serialize_record(root)?;

  if root.len() != 34 {
    return Err(CodecError::Message(format!(
      "The root directory record takes {} bytes, and the descriptor reserves 34 for it",
      root.len()
    )));
  }

  bytes[156..190].copy_from_slice(&root);

  put_text(&mut bytes, 190, 128, &primary.volume_set_identifier)?;
  put_text(&mut bytes, 318, 128, &primary.publisher_identifier)?;
  put_text(&mut bytes, 446, 128, &primary.preparer_identifier)?;
  put_text(&mut bytes, 574, 128, &primary.application_identifier)?;
  put_text(&mut bytes, 702, 37, &primary.copyright_file_identifier)?;
  put_text(&mut bytes, 739, 37, &primary.abstract_file_identifier)?;
  put_text(&mut bytes, 776, 37, &primary.bibliographic_file_identifier)?;

  put_date(&mut bytes, 813, &primary.volume_creation_date)?;
  put_date(&mut bytes, 830, &primary.volume_modification_date)?;
  put_date(&mut bytes, 847, &primary.volume_expiration_date)?;
  put_date(&mut bytes, 864, &primary.volume_effective_date)?;

  bytes[PRIMARY_FILE_STRUCTURE_VERSION] = primary.file_structure_version;

  put_application_use(&mut bytes, &primary.application_use)?;

  Ok(bytes)
}

/// Writes a descriptor's Application Use area.
fn put_application_use(bytes: &mut [u8], application_use: &Hex) -> Result<(), CodecError> {
  if application_use.len() > PRIMARY_APPLICATION_USE_LEN {
    return Err(CodecError::Message(format!(
      "The image's Application Use area is {} bytes, and ISO 9660 reserves \
       {PRIMARY_APPLICATION_USE_LEN}",
      application_use.len()
    )));
  }

  bytes[PRIMARY_APPLICATION_USE..PRIMARY_APPLICATION_USE + application_use.len()]
    .copy_from_slice(application_use.as_bytes());

  Ok(())
}

/// The bytes of a path table, written the way round the table asks for.
pub(crate) fn serialize_path_table(path_table: &PathTable, big_endian: bool) -> Vec<u8> {
  let mut bytes = Vec::with_capacity(path_table.size());

  for entry in &path_table.entries {
    let name = entry.name.as_bytes();

    bytes.push(name.len() as u8);
    bytes.push(0);

    bytes.extend_from_slice(&if big_endian {
      entry.extent.to_be_bytes()
    } else {
      entry.extent.to_le_bytes()
    });

    bytes.extend_from_slice(&if big_endian {
      entry.parent.to_be_bytes()
    } else {
      entry.parent.to_le_bytes()
    });

    bytes.extend_from_slice(name);

    // A path table entry is a fixed part of eight bytes and an identifier, and
    // an odd identifier takes a pad byte to keep the entry even.
    if !name.len().is_multiple_of(2) {
      bytes.push(0);
    }
  }

  bytes
}

/// The bytes of one directory record.
pub(crate) fn serialize_record(record: &Record) -> Result<Vec<u8>, CodecError> {
  let name = record.name.as_bytes();

  if name.is_empty() {
    return Err(CodecError::Message(
      "A directory record carries an empty file identifier".to_owned(),
    ));
  }

  if name.len() > u8::MAX as usize {
    return Err(CodecError::Message(format!(
      "The file identifier '{}' is longer than a directory record can hold",
      record.name
    )));
  }

  if record.date.len() != 7 {
    return Err(CodecError::Message(format!(
      "The record for '{}' carries a date of {} bytes rather than the 7 ISO 9660 records",
      record.name,
      record.date.len()
    )));
  }

  let pad = usize::from(name.len().is_multiple_of(2));
  let length = RECORD_HEADER + name.len() + pad + record.system_use.len();

  if length > u8::MAX as usize {
    return Err(CodecError::Message(format!(
      "The record for '{}' is longer than a directory record can be",
      record.name
    )));
  }

  let mut bytes = vec![0u8; length];

  bytes[0] = length as u8;
  put_both_ended(&mut bytes, 2, 4, record.extent)?;
  put_both_ended(&mut bytes, 10, 4, record.length)?;
  bytes[18..25].copy_from_slice(record.date.as_bytes());
  bytes[25] = record.flags;
  put_both_ended(&mut bytes, 28, 2, u32::from(record.volume_sequence_number))?;
  bytes[32] = name.len() as u8;
  bytes[33..33 + name.len()].copy_from_slice(name);
  bytes[33 + name.len() + pad..].copy_from_slice(record.system_use.as_bytes());

  Ok(bytes)
}

/// The bytes of one directory, its records laid out in the blocks it holds.
pub(crate) fn serialize_directory(directory: &Directory) -> Result<Vec<u8>, CodecError> {
  let length = directory.length as usize;
  let mut bytes = Vec::new();

  bytes.try_reserve_exact(length).map_err(|_| {
    CodecError::Message(format!(
      "The directory '{}' declares {} bytes, more than this machine can hold",
      display_path(&directory.path),
      directory.length
    ))
  })?;

  for held in &directory.records {
    let record = serialize_record(held)?;

    if held.pad.is_empty() {
      // A record the image never held goes right after the one before it,
      // unless it would straddle a block boundary, which ISO 9660 does not let
      // it do: the rest of the block holds nothing and the record goes on in
      // the next one.
      let used = bytes.len() % BLOCK_SIZE;

      if used + record.len() > BLOCK_SIZE {
        bytes.resize(bytes.len() + (BLOCK_SIZE - used), 0);
      }
    } else {
      // What the image held between the two records goes back as it was, which
      // is what puts this record in the block it sat in.
      bytes.extend_from_slice(held.pad.as_bytes());
    }

    if bytes.len() + record.len() > length {
      return Err(CodecError::Message(format!(
        "The records of the directory '{}' no longer fit in the {} bytes its extent holds",
        display_path(&directory.path),
        directory.length
      )));
    }

    bytes.extend_from_slice(&record);
  }

  bytes.resize(length, 0);

  Ok(bytes)
}

/// Where each of a directory's records sits in its extent, and how long it is.
///
/// Its place is the one [`serialize_directory`] writes it to: a record carries
/// the bytes the image held before it, or follows the record before it and goes
/// on in the next block when it would not fit in the room that is left.
pub(crate) fn record_offsets(records: &[Record]) -> Result<Vec<(usize, usize)>, CodecError> {
  let mut offsets = Vec::with_capacity(records.len());
  let mut used = 0;

  for record in records {
    let size = serialize_record(record)?.len();

    if record.pad.is_empty() {
      let filled = used % BLOCK_SIZE;

      if filled + size > BLOCK_SIZE {
        used += BLOCK_SIZE - filled;
      }
    } else {
      used += record.pad.len();
    }

    offsets.push((used, size));
    used += size;
  }

  Ok(offsets)
}

/// The image's path tables, from the addresses its descriptor gives.
fn parse_path_tables(
  image: &[u8],
  descriptor: &[u8],
  coverage: &mut Coverage,
) -> Result<PathTable, CodecError> {
  let size =
    usize::try_from(read_both_ended(descriptor, 132, 4, "Path Table Size")?).map_err(|_| {
      CodecError::Message("The path tables are larger than this machine can hold".to_owned())
    })?;

  let l_sector = u32::from_le_bytes(descriptor[140..144].try_into().expect("four bytes"));
  let optional_l_sector = u32::from_le_bytes(descriptor[144..148].try_into().expect("four bytes"));
  let m_sector = u32::from_be_bytes(descriptor[148..152].try_into().expect("four bytes"));
  let optional_m_sector = u32::from_be_bytes(descriptor[152..156].try_into().expect("four bytes"));

  let little = parse_path_table(image, l_sector, size, false, "the type L path table")?;
  let big = parse_path_table(image, m_sector, size, true, "the type M path table")?;

  if little != big {
    return Err(CodecError::Message(
      "The image's two path tables hold different directories, which this codec does not rebuild"
        .to_owned(),
    ));
  }

  coverage.cover(
    l_sector as usize * BLOCK_SIZE,
    size,
    "the type L path table".to_owned(),
  )?;
  coverage.cover(
    m_sector as usize * BLOCK_SIZE,
    size,
    "the type M path table".to_owned(),
  )?;

  // A second copy of either table is left where it lies: its bytes are opaque to
  // a rebuild, which will not move anything while one is there.
  Ok(PathTable {
    l_sector,
    m_sector,
    optional_l_sector,
    optional_m_sector,
    entries: little,
  })
}

/// One path table's entries.
fn parse_path_table(
  image: &[u8],
  sector: u32,
  size: usize,
  big_endian: bool,
  what: &str,
) -> Result<Vec<PathTableEntry>, CodecError> {
  let start = sector as usize * BLOCK_SIZE;

  let table = image
    .get(start..start + size)
    .ok_or_else(|| CodecError::Message(format!("The image's {what} runs past the image")))?;

  let mut entries = Vec::new();
  let mut offset = 0;

  while offset < size {
    let name_len = table[offset] as usize;
    let entry_size = 8 + name_len + usize::from(!name_len.is_multiple_of(2));

    if name_len == 0 || offset + entry_size > size {
      return Err(CodecError::Message(format!(
        "The image's {what} does not hold whole entries: its descriptor declares {size} bytes and \
         only {offset} of them account for entries"
      )));
    }

    if table[offset + 1] != 0 {
      return Err(CodecError::Message(format!(
        "The image's {what} holds a directory with an extended attribute record, which this codec \
         does not rebuild"
      )));
    }

    let extent = if big_endian {
      u32::from_be_bytes(
        table[offset + 2..offset + 6]
          .try_into()
          .expect("four bytes"),
      )
    } else {
      u32::from_le_bytes(
        table[offset + 2..offset + 6]
          .try_into()
          .expect("four bytes"),
      )
    };

    let parent = if big_endian {
      u16::from_be_bytes(table[offset + 6..offset + 8].try_into().expect("two bytes"))
    } else {
      u16::from_le_bytes(table[offset + 6..offset + 8].try_into().expect("two bytes"))
    };

    let name = &table[offset + 8..offset + 8 + name_len];

    entries.push(PathTableEntry {
      name: std::str::from_utf8(name)
        .map_err(|err| {
          CodecError::Message(format!(
            "A directory identifier in the image's {what} is not valid text: {err}"
          ))
        })?
        .to_owned(),
      extent,
      parent,
    });

    offset += entry_size;
  }

  Ok(entries)
}

/// The Primary Volume Descriptor's fields, as the image holds them.
fn parse_primary(descriptor: &[u8]) -> Result<Primary, CodecError> {
  let block_size = read_both_ended(descriptor, 128, 2, "Logical Block Size")?;

  if block_size as usize != BLOCK_SIZE {
    return Err(CodecError::Message(format!(
      "The image's logical block size is {block_size} bytes, and this codec works in \
       {BLOCK_SIZE}-byte blocks"
    )));
  }

  // The File Structure Version is carried through rather than validated:
  // ECMA-119 prescribes 1 for a primary descriptor, but the field names the
  // structure version of the records and path tables — which this codec has
  // just read as they are — and changes no other byte's meaning. An image
  // declaring another version rebuilds by writing back what it declared.
  Ok(Primary {
    file_structure_version: descriptor[PRIMARY_FILE_STRUCTURE_VERSION],
    system_identifier: read_text(descriptor, 8, 32, "System Identifier")?,
    volume_set_identifier: read_text(descriptor, 190, 128, "Volume Set Identifier")?,
    publisher_identifier: read_text(descriptor, 318, 128, "Publisher Identifier")?,
    preparer_identifier: read_text(descriptor, 446, 128, "Preparer Identifier")?,
    application_identifier: read_text(descriptor, 574, 128, "Application Identifier")?,
    copyright_file_identifier: read_text(descriptor, 702, 37, "Copyright File Identifier")?,
    abstract_file_identifier: read_text(descriptor, 739, 37, "Abstract File Identifier")?,
    bibliographic_file_identifier: read_text(descriptor, 776, 37, "Bibliographic File Identifier")?,
    volume_creation_date: read_date(descriptor, 813),
    volume_modification_date: read_date(descriptor, 830),
    volume_expiration_date: read_date(descriptor, 847),
    volume_effective_date: read_date(descriptor, 864),
    volume_set_size: read_both_ended(descriptor, 120, 2, "Volume Set Size")? as u16,
    volume_sequence_number: read_both_ended(descriptor, 124, 2, "Volume Sequence Number")? as u16,
    logical_block_size: block_size as u16,
    application_use: {
      let field =
        &descriptor[PRIMARY_APPLICATION_USE..PRIMARY_APPLICATION_USE + PRIMARY_APPLICATION_USE_LEN];

      // What the writer left in the area, with the zeroes it padded the end with
      // trimmed: a rebuild writes the area back padded with zeroes, so the same
      // bytes come out, and metadata that spelled out the whole area would say
      // nothing in 512 bytes for every image whose writer left it empty.
      let end = field
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);

      Hex::from(&field[..end])
    },
  })
}

/// One directory record, as the image holds it.
fn parse_record(bytes: &[u8], what: &str) -> Result<Record, CodecError> {
  if bytes.len() < RECORD_HEADER + 1 {
    return Err(CodecError::Message(format!(
      "{what} is shorter than an ISO 9660 directory record"
    )));
  }

  if bytes[1] != 0 {
    return Err(CodecError::Message(format!(
      "{what} has an extended attribute record, which this codec does not rebuild"
    )));
  }

  if bytes[26] != 0 || bytes[27] != 0 {
    return Err(CodecError::Message(format!(
      "{what} describes an interleaved file, which this codec does not rebuild"
    )));
  }

  let name_len = bytes[32] as usize;
  let name_end = RECORD_HEADER + name_len + usize::from(name_len.is_multiple_of(2));

  if name_len == 0 || name_end > bytes.len() {
    return Err(CodecError::Message(format!(
      "{what} carries a file identifier that does not fit in it"
    )));
  }

  let record = Record {
    name: std::str::from_utf8(&bytes[33..33 + name_len])
      .map_err(|err| {
        CodecError::Message(format!(
          "{what} carries a file identifier that is not valid text: {err}"
        ))
      })?
      .to_owned(),
    extent: read_both_ended(bytes, 2, 4, &format!("the extent {what} gives"))?,
    length: read_both_ended(bytes, 10, 4, &format!("the data length {what} gives"))?,
    flags: bytes[25],
    date: Hex::from(&bytes[18..25]),
    volume_sequence_number: read_both_ended(
      bytes,
      28,
      2,
      &format!("the volume sequence number {what} gives"),
    )? as u16,
    system_use: Hex::from(&bytes[name_end..]),
    // What the directory held before it, which the walk fills in.
    pad: Hex::default(),
  };

  if let Some(unsupported) = record.unsupported() {
    return Err(CodecError::Message(format!(
      "{what} describes {unsupported}, which this codec does not rebuild"
    )));
  }

  if record.size() != bytes.len() {
    return Err(CodecError::Message(format!(
      "{what} is {} bytes, and its own fields account for {}",
      bytes.len(),
      record.size()
    )));
  }

  Ok(record)
}

/// Walks the image's directory tree.
fn walk(
  image: &[u8],
  root: &Record,
  out_of_bounds: OutOfBoundsEntries,
  read_file_contents: bool,
  coverage: &mut Coverage,
) -> Result<(Vec<Directory>, Vec<FileEntry>), CodecError> {
  let mut directories = Vec::new();
  let mut files = Vec::new();
  let mut pending = VecDeque::from([(String::new(), root.clone())]);
  let mut visited = BTreeSet::new();

  while let Some((path, record)) = pending.pop_front() {
    if directories.len() >= MAX_DIRECTORIES {
      return Err(CodecError::Message(format!(
        "ISO 9660 image contains more than {MAX_DIRECTORIES} directories"
      )));
    }

    if !visited.insert(record.extent) {
      return Err(CodecError::Message(format!(
        "The image's directories do not form a tree: the one at block {} appears twice",
        record.extent
      )));
    }

    let length = record.length as usize;

    if length == 0 {
      return Err(CodecError::Message(format!(
        "The directory '{}' has no recorded directory data",
        display_path(&path)
      )));
    }

    let start = record.extent as usize * BLOCK_SIZE;

    let data = image.get(start..start + length).ok_or_else(|| {
      CodecError::Message(format!(
        "The directory '{}' holds blocks the image does not: it starts at block {} and spans \
         {length} bytes",
        display_path(&path),
        record.extent
      ))
    })?;

    let mut records = Vec::new();
    let mut offset = 0;
    let mut pad = Hex::default();

    while offset < length {
      let size = data[offset] as usize;

      // A record length of zero says the rest of the block holds nothing, and
      // the directory goes on in the next one. The bytes that are left are
      // what the size of this directory's records left over, and they are
      // recorded so that the record that follows goes back where it was.
      if size == 0 {
        let next = ((offset / BLOCK_SIZE + 1) * BLOCK_SIZE).min(length);

        pad = Hex::from(&data[offset..next]);
        offset = next;
        continue;
      }

      let what = format!(
        "The record at offset {offset} of the directory '{}'",
        display_path(&path)
      );

      // ISO 9660 does not let a record cross a logical block boundary: the
      // rebuild pads such a record on to the next block, so an image holding
      // one cannot be written back where it stood and is refused rather than
      // silently moved.
      if offset % BLOCK_SIZE + size > BLOCK_SIZE {
        return Err(CodecError::Message(format!(
          "{what} straddles a logical block boundary, which ISO 9660 does not allow"
        )));
      }

      let bytes = data
        .get(offset..offset + size)
        .ok_or_else(|| CodecError::Message(format!("{what} runs past the end of the directory")))?;
      let mut held = parse_record(bytes, &what)?;
      let padding = std::mem::take(&mut pad);
      let padding_len = padding.len();

      held.pad = padding;

      if serialize_record(&held)? != bytes {
        return Err(CodecError::Message(format!(
          "{what} is not one this codec can write back as it stands"
        )));
      }

      coverage.cover(
        start + offset - padding_len,
        padding_len + size,
        format!("{what} of the image and what precedes it"),
      )?;

      offset += size;

      if held.is_directory() {
        // Every directory carries entries for itself and for its parent, which
        // name no file of their own.
        if held.name == "\u{0}" || held.name == "\u{1}" {
          records.push(held);
          continue;
        }

        pending.push_back((join(&path, &held.name), held.clone()));
        records.push(held);
        continue;
      }

      let name = join(&path, &strip_version(&held.name, &path)?);
      let available = held.in_image(image.len());

      if available < held.length as usize {
        match out_of_bounds {
          OutOfBoundsEntries::Error => {
            return Err(CodecError::Message(format!(
              "The file '{name}' lies outside the image, or extends past its end: it declares {} \
               bytes at block {}, which an image of {} bytes does not hold",
              held.length,
              held.extent,
              image.len()
            )));
          }
          OutOfBoundsEntries::Ignore => {
            debug!("Skipping out-of-bounds ISO 9660 entry '{name}'");
            continue;
          }
          OutOfBoundsEntries::Preserve => {
            debug!(
              "Preserving '{name}', whose contents the image does not hold in full: {available} of \
               {} bytes",
              held.length
            );
          }
        }
      }

      // A record whose data lies wholly outside the image is kept, and nothing
      // carries its contents: handing back an empty file for a file that is not
      // empty would say the disc holds nothing where it holds another track.
      if available > 0 || held.length == 0 {
        let start = held.extent as usize * BLOCK_SIZE;
        let data = if read_file_contents {
          image
            .get(start..start + available)
            .map(<[u8]>::to_vec)
            .unwrap_or_default()
        } else {
          Vec::new()
        };

        if available > 0 {
          coverage.cover(start, available, format!("the file '{name}'"))?;
        }

        files.push(FileEntry {
          name,
          extent: held.extent,
          length: available,
          data,
        });
      }

      records.push(held);
    }

    directories.push(Directory {
      path,
      extent: record.extent,
      length: record.length,
      records,
    });
  }

  Ok((directories, files))
}

/// The name a file's artifact is given: its identifier with the version suffix
/// the writer supplies again taken off.
fn strip_version(identifier: &str, path: &str) -> Result<String, CodecError> {
  let name = identifier
    .split_once(';')
    .map_or(identifier, |(name, _)| name);

  if name.is_empty() {
    return Err(CodecError::Message(format!(
      "The directory '{}' holds a file whose identifier is nothing but a version",
      display_path(path)
    )));
  }

  Ok(name.to_owned())
}

/// A path below a directory.
fn join(path: &str, name: &str) -> String {
  if path.is_empty() {
    name.to_owned()
  } else {
    format!("{path}/{name}")
  }
}

/// How a directory is named in a message, the root having no path of its own.
fn display_path(path: &str) -> String {
  if path.is_empty() {
    "/".to_owned()
  } else {
    format!("/{path}")
  }
}

/// Checks that what the codec would write for the descriptor is what the image
/// holds, so that a rebuild cannot quietly differ from the image it read.
fn verify_primary(
  descriptor: &[u8],
  volume_identifier: &str,
  layout: &Layout,
) -> Result<(), CodecError> {
  let root = layout
    .root()
    .and_then(Directory::own_record)
    .ok_or_else(|| CodecError::Message("The image's tree holds no root directory".to_owned()))?;

  let mut root = root.clone();
  root.system_use = Hex::default();

  // What the image declares is what a rebuild of it writes.
  let written = serialize_primary(layout, volume_identifier, &root, layout.volume_space_size)?;

  for (name, offset, width) in PRIMARY_FIELDS {
    if written[*offset..*offset + *width] != descriptor[*offset..*offset + *width] {
      return Err(CodecError::Message(format!(
        "The image's Primary Volume Descriptor holds {name} that this codec cannot write back as it \
         stands"
      )));
    }
  }

  Ok(())
}

/// Checks that what the codec would write for the path tables is what the image
/// holds.
fn verify_path_table(image: &[u8], path_table: &PathTable) -> Result<(), CodecError> {
  let size = path_table.size();

  for (sector, big_endian) in [
    (path_table.l_sector, false),
    (path_table.m_sector, true),
  ] {
    let start = sector as usize * BLOCK_SIZE;
    let written = serialize_path_table(path_table, big_endian);

    if image.get(start..start + size) != Some(&written[..]) {
      return Err(CodecError::Message(
        "The image's path tables do not hold what this codec would write for them".to_owned(),
      ));
    }
  }

  Ok(())
}

/// A text field of a descriptor, without the padding ISO 9660 mandates.
fn read_text(bytes: &[u8], offset: usize, width: usize, what: &str) -> Result<String, CodecError> {
  let field = bytes
    .get(offset..offset + width)
    .ok_or_else(|| CodecError::Message(format!("A volume descriptor holds no {what}")))?;

  let trimmed = match field.iter().rposition(|byte| *byte != b' ') {
    Some(last) => &field[..=last],
    None => &[][..],
  };

  std::str::from_utf8(trimmed)
    .map(str::to_owned)
    .map_err(|err| {
      CodecError::Message(format!(
        "The image's {what} is not text and cannot be written back: {err}"
      ))
    })
}

/// A date field of a descriptor, which a writer may leave zeroed.
fn read_date(bytes: &[u8], offset: usize) -> Hex {
  let field = &bytes[offset..offset + PRIMARY_DATE];

  if field.iter().all(|byte| *byte == 0) {
    Hex::default()
  } else {
    Hex::from(field)
  }
}

/// A number ISO 9660 writes twice, once each way round.
fn read_both_ended(
  bytes: &[u8],
  offset: usize,
  width: usize,
  what: &str,
) -> Result<u32, CodecError> {
  let little = &bytes[offset..offset + width];
  let big = &bytes[offset + width..offset + width * 2];

  let little = match width {
    4 => u32::from_le_bytes(little.try_into().expect("four bytes")),
    _ => u32::from(u16::from_le_bytes(little.try_into().expect("two bytes"))),
  };

  let big = match width {
    4 => u32::from_be_bytes(big.try_into().expect("four bytes")),
    _ => u32::from(u16::from_be_bytes(big.try_into().expect("two bytes"))),
  };

  if little != big {
    return Err(CodecError::Message(format!(
      "The image's {what} is written as {little} one way round and {big} the other, which this \
       codec does not rebuild"
    )));
  }

  Ok(little)
}

/// Writes a number ISO 9660 keeps twice, once each way round.
fn put_both_ended(
  bytes: &mut [u8],
  offset: usize,
  width: usize,
  value: u32,
) -> Result<(), CodecError> {
  let mut little = [0u8; 4];
  let mut big = [0u8; 4];

  match width {
    4 => {
      little = value.to_le_bytes();
      big = value.to_be_bytes();
    }
    2 => {
      let value = u16::try_from(value).map_err(|_| {
        CodecError::Message(format!(
          "{value} is too large for a two-byte ISO 9660 field"
        ))
      })?;

      little[..2].copy_from_slice(&value.to_le_bytes());
      big[..2].copy_from_slice(&value.to_be_bytes());
    }
    _ => {
      return Err(CodecError::Message(format!(
        "ISO 9660 writes numbers in two or four bytes, not {width}"
      )));
    }
  }

  bytes[offset..offset + width].copy_from_slice(&little[..width]);
  bytes[offset + width..offset + width * 2].copy_from_slice(&big[..width]);

  Ok(())
}

/// Writes a text field, padded the way ISO 9660 asks for.
fn put_text(bytes: &mut [u8], offset: usize, width: usize, value: &str) -> Result<(), CodecError> {
  if value.len() > width {
    return Err(CodecError::Message(format!(
      "'{value}' does not fit in the {width} bytes ISO 9660 reserves for it"
    )));
  }

  bytes[offset..offset + width].fill(b' ');
  bytes[offset..offset + value.len()].copy_from_slice(value.as_bytes());

  Ok(())
}

/// Writes a volume date, which an empty one leaves zeroed.
fn put_date(bytes: &mut [u8], offset: usize, date: &Hex) -> Result<(), CodecError> {
  if date.is_empty() {
    return Ok(());
  }

  if date.len() != PRIMARY_DATE {
    return Err(CodecError::Message(format!(
      "A volume date of {} bytes cannot be written as the {PRIMARY_DATE} bytes ISO 9660 dates take",
      date.len()
    )));
  }

  bytes[offset..offset + PRIMARY_DATE].copy_from_slice(date.as_bytes());

  Ok(())
}

/// The bytes of an image that belong to a structure the codec models.
///
/// What is left over is what a rebuild preserves without interpreting, so
/// finding it means knowing exactly which bytes the model accounts for.
struct Coverage {
  image_len: usize,
  ranges: Vec<(ByteRange, String)>,
}

impl Coverage {
  fn new(image_len: usize) -> Self {
    Self {
      image_len,
      ranges: Vec::new(),
    }
  }

  fn cover(&mut self, start: usize, len: usize, what: String) -> Result<(), CodecError> {
    let end = start
      .checked_add(len)
      .filter(|end| *end <= self.image_len)
      .ok_or_else(|| CodecError::Message(format!("{what} runs past the end of the image")))?;

    if len > 0 {
      let range = ByteRange::new(start as u64, (end - start) as u64)
        .ok_or_else(|| CodecError::Message(format!("{what} byte range overflows")))?;
      self.ranges.push((range, what));
    }

    Ok(())
  }

  /// Two structures claiming the same bytes, if any.
  fn overlap(&self) -> Option<(String, String)> {
    let mut ranges = self.ranges.clone();
    ranges.sort_by_key(|(range, _)| (range.start, range.end));

    ranges.windows(2).find_map(|pair| {
      let [
        (left_range, left),
        (right_range, right),
      ] = pair
      else {
        return None;
      };

      left_range
        .overlaps(*right_range)
        .then(|| (left.clone(), right.clone()))
    })
  }

  /// The runs of bytes no structure accounts for and that are not zero.
  ///
  /// Zeroes are not recorded: an image is written from a buffer of them, so what
  /// a rebuild has to be told is where the bytes that are not zero belong.
  fn opaque_runs(&self, image: &[u8]) -> Vec<Opaque> {
    let mut ranges = self.ranges.clone();
    ranges.sort_by_key(|(range, _)| (range.start, range.end));

    let mut runs = Vec::new();
    let mut cursor = 0;

    for (range, _) in ranges {
      let start = range.start as usize;
      let end = range.end as usize;
      scan(image, cursor, start, &mut runs);
      cursor = cursor.max(end);
    }

    scan(image, cursor, self.image_len, &mut runs);

    runs
  }
}

/// The runs of bytes that are not zero between two offsets.
fn scan(image: &[u8], from: usize, to: usize, runs: &mut Vec<Opaque>) {
  let mut index = from;

  while index < to {
    if image[index] == 0 {
      index += 1;
      continue;
    }

    let start = index;

    while index < to && image[index] != 0 {
      index += 1;
    }

    runs.push(Opaque {
      offset: start as u64,
      data: Hex::from(&image[start..index]),
    });
  }
}
