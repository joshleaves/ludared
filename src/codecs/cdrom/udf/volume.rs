//! UDF descriptor, directory, and file-entry parsing.
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::metadata::DataExtent;
use crate::codecs::cdrom::common_disk::BLOCK_SIZE;
use crate::codecs::cdrom::common_disk::ByteRange;
use crate::codecs::cdrom::common_disk::block_offset;
use crate::codecs::cdrom::iso9660::volume as iso_volume;
use crate::codecs::errors::CodecError;

const MAX_VDS_DESCRIPTORS: usize = 256;
const MAX_DIRECTORY_DEPTH: usize = 128;
pub(super) const TAG_ANCHOR: u16 = 2;
pub(super) const TAG_PRIMARY_VOLUME: u16 = 1;
pub(super) const TAG_PARTITION: u16 = 5;
pub(super) const TAG_LOGICAL_VOLUME: u16 = 6;
pub(super) const TAG_FILE_SET: u16 = 256;
pub(super) const TAG_FILE_IDENTIFIER: u16 = 257;
pub(super) const TAG_FILE_ENTRY: u16 = 261;

#[derive(Debug)]
pub(super) struct UdfFile {
  pub(super) path: String,
  pub(super) aliases: Vec<String>,
  pub(super) data: Vec<u8>,
  pub(super) extents: Vec<DataExtent>,
  pub(super) file_entry_offsets: Vec<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Partition {
  number: u16,
  start: u32,
  length: u32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LongAddress {
  block: u32,
  partition_map: u16,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PartitionMap {
  partition_number: u16,
}

#[derive(Debug)]
pub(super) struct UdfReading {
  pub(super) revision: u16,
  pub(super) files: Vec<UdfFile>,
}

#[derive(Debug, Clone)]
pub(super) struct ProtectedRange {
  offset: u64,
  length: u64,
  description: String,
}

pub(super) fn has_udf_vrs(data: &[u8]) -> bool {
  data
    .as_chunks::<BLOCK_SIZE>()
    .0
    .iter()
    .skip(16)
    .take(16)
    .any(|block| matches!(block.get(1..6), Some(b"NSR02" | b"NSR03")))
}

pub(super) fn valid_anchor(data: &[u8]) -> bool {
  data
    .get(256 * BLOCK_SIZE..257 * BLOCK_SIZE)
    .is_some_and(|block| check_tag(block, TAG_ANCHOR).is_ok())
}

pub(super) fn read_udf(image: &[u8]) -> Result<UdfReading, CodecError> {
  if image.is_empty() || !image.len().is_multiple_of(BLOCK_SIZE) {
    return Err(CodecError::Message(format!(
      "A UDF image must contain whole {BLOCK_SIZE}-byte logical blocks; this one is {} bytes",
      image.len()
    )));
  }

  let anchor = block(image, 256)?;
  check_tag(anchor, TAG_ANCHOR)?;
  let (sequence_len, sequence_location) = extent_ad(anchor, 16)?;
  let sequence_blocks = sequence_len.div_ceil(BLOCK_SIZE as u32);
  if sequence_blocks == 0 || sequence_blocks > MAX_VDS_DESCRIPTORS as u32 {
    return Err(CodecError::Message(format!(
      "UDF volume descriptor sequence spans {sequence_blocks} blocks; supported sequences span 1 through {MAX_VDS_DESCRIPTORS} blocks"
    )));
  }
  let mut partitions = BTreeMap::new();
  let mut maps = Vec::new();
  let mut lvd = None;
  let mut protected = vec![
    ProtectedRange {
      offset: 16 * BLOCK_SIZE as u64,
      length: 16 * BLOCK_SIZE as u64,
      description: "the UDF volume recognition sequence".to_owned(),
    },
    ProtectedRange {
      offset: 256 * BLOCK_SIZE as u64,
      length: BLOCK_SIZE as u64,
      description: "the UDF anchor descriptor".to_owned(),
    },
  ];
  let mut found_terminator = false;

  for index in 0..sequence_blocks {
    let descriptor_block = sequence_location.checked_add(index).ok_or_else(|| {
      CodecError::Message("UDF volume descriptor sequence location overflows".to_owned())
    })?;
    let descriptor = block(image, descriptor_block)?;
    let tag = tag_id(descriptor)?;
    protected.push(ProtectedRange {
      offset: u64::from(descriptor_block) * BLOCK_SIZE as u64,
      length: BLOCK_SIZE as u64,
      description: format!("UDF volume descriptor at block {descriptor_block}"),
    });
    if tag == 8 {
      check_tag(descriptor, 8)?;
      found_terminator = true;
      break;
    }
    if !matches!(
      tag,
      TAG_PRIMARY_VOLUME | 4 | TAG_PARTITION | TAG_LOGICAL_VOLUME | 7
    ) {
      return Err(CodecError::Message(format!(
        "UDF volume descriptor sequence contains unsupported tag {tag} at block {descriptor_block}"
      )));
    }
    check_tag(descriptor, tag)?;
    match tag {
      TAG_PARTITION => {
        let number = u16_at(descriptor, 22)?;
        let start = u32_at(descriptor, 188)?;
        let length = u32_at(descriptor, 192)?;
        partitions.insert(
          number,
          Partition {
            number,
            start,
            length,
          },
        );
      }
      TAG_LOGICAL_VOLUME => {
        let map_len = u32_at(descriptor, 264)? as usize;
        let map_count = u32_at(descriptor, 268)? as usize;
        let map_start = 440usize;
        let map_end = map_start.checked_add(map_len).ok_or_else(|| {
          CodecError::Message("UDF partition map table length overflows".to_owned())
        })?;
        let table = descriptor.get(map_start..map_end).ok_or_else(|| {
          CodecError::Message("UDF partition map table lies outside its descriptor".to_owned())
        })?;
        let mut offset = 0usize;
        maps.clear();
        for _ in 0..map_count {
          let kind = *table.get(offset).ok_or_else(|| {
            CodecError::Message("UDF partition map table is truncated".to_owned())
          })?;
          let length = *table
            .get(offset + 1)
            .ok_or_else(|| CodecError::Message("UDF partition map table is truncated".to_owned()))?
            as usize;
          let map = table.get(offset..offset + length).ok_or_else(|| {
            CodecError::Message("UDF partition map has an invalid length".to_owned())
          })?;
          if kind != 1 || length != 6 {
            return Err(CodecError::Message(format!(
              "UDF partition map type {kind} with length {length} is unsupported; only physical Type-1 maps are supported"
            )));
          }
          maps.push(PartitionMap {
            partition_number: u16_at(map, 4)?,
          });
          offset += length;
        }
        if offset != map_len {
          return Err(CodecError::Message(format!(
            "UDF partition maps use {offset} bytes, but the descriptor declares {map_len}"
          )));
        }
        lvd = Some((descriptor.to_vec(), long_address(descriptor, 248)?));
      }
      _ => {}
    }
  }

  if !found_terminator {
    return Err(CodecError::Message(
      "UDF volume descriptor sequence has no terminating descriptor".to_owned(),
    ));
  }

  let (lvd, fsd_address) = lvd.ok_or_else(|| {
    CodecError::Message(
      "UDF volume descriptor sequence contains no Logical Volume Descriptor".to_owned(),
    )
  })?;
  let revision = u16_at(&lvd, 240)?;
  if !(0x0102..=0x0150).contains(&revision) {
    return Err(CodecError::Message(format!(
      "UDF revision {revision:#06x} is outside the supported range 1.02 through 1.50"
    )));
  }
  let logical_block_size = u32_at(&lvd, 212)?;
  if logical_block_size != BLOCK_SIZE as u32 {
    return Err(CodecError::Message(format!(
      "UDF logical block size is {logical_block_size}, but this codec requires {BLOCK_SIZE}"
    )));
  }
  if maps.len() != 1 {
    return Err(CodecError::Message(format!(
      "UDF image has {} partition maps; this codec supports one",
      maps.len()
    )));
  }
  let partition = *partitions.get(&maps[0].partition_number).ok_or_else(|| {
    CodecError::Message(format!(
      "UDF partition map refers to missing partition {}",
      maps[0].partition_number
    ))
  })?;
  if partition.length == 0 {
    return Err(CodecError::Message("UDF partition is empty".to_owned()));
  }
  let partition_end = u64::from(partition.start) + u64::from(partition.length);
  if partition_end > (image.len() / BLOCK_SIZE) as u64 {
    return Err(CodecError::Message(format!(
      "UDF partition {} ends at block {partition_end}, past the image's {} blocks",
      partition.number,
      image.len() / BLOCK_SIZE
    )));
  }

  let fsd_offset = resolve_address(image, partition, fsd_address)?;
  protected.push(ProtectedRange {
    offset: fsd_offset as u64,
    length: BLOCK_SIZE as u64,
    description: "the UDF File Set Descriptor".to_owned(),
  });
  let fsd = descriptor_at(image, fsd_offset, TAG_FILE_SET)?;
  if fsd
    .get(456..472)
    .is_some_and(|address| address.iter().any(|byte| *byte != 0))
  {
    return Err(CodecError::Message(
      "UDF File Set Descriptor uses an unsupported system stream directory".to_owned(),
    ));
  }
  let root_address = long_address(fsd, 400)?;
  let mut files = Vec::new();
  let mut visited = BTreeSet::new();
  walk_directory(
    image,
    partition,
    root_address,
    "",
    0,
    &mut visited,
    &mut files,
    &mut protected,
  )?;

  // A hard link is identified by a shared ICB/File Entry. Equal data extents
  // alone do not establish identity, so distinct File Entries sharing bytes
  // are rejected below instead of silently merging two files.
  files.sort_by(|left, right| {
    left
      .extents
      .first()
      .map_or(u64::MAX, |extent| extent.offset)
      .cmp(
        &right
          .extents
          .first()
          .map_or(u64::MAX, |extent| extent.offset),
      )
  });

  let mut unique: Vec<UdfFile> = Vec::new();
  for file in files {
    if let Some(existing) = unique.iter_mut().find(|existing| {
      file
        .file_entry_offsets
        .iter()
        .any(|offset| existing.file_entry_offsets.contains(offset))
    }) {
      if existing.extents != file.extents || existing.data.len() != file.data.len() {
        return Err(CodecError::Message(format!(
          "UDF hard-link paths '{}' and '{}' share a File Entry but disagree about its allocation",
          existing.path, file.path
        )));
      }
      if existing.data != file.data {
        return Err(CodecError::Message(format!(
          "UDF hard-link paths '{}' and '{}' share a File Entry but yield different data",
          existing.path, file.path
        )));
      }
      existing.aliases.extend(file.aliases);
      continue;
    }
    unique.push(file);
  }

  validate_udf_allocations(&unique, &protected)?;

  Ok(UdfReading {
    revision,
    files: unique,
  })
}

#[allow(clippy::too_many_arguments)] // The recursive walk carries shared traversal and protection state.
fn walk_directory(
  image: &[u8],
  partition: Partition,
  address: LongAddress,
  path: &str,
  depth: usize,
  visited: &mut BTreeSet<(u32, u16)>,
  files: &mut Vec<UdfFile>,
  protected: &mut Vec<ProtectedRange>,
) -> Result<(), CodecError> {
  if depth > MAX_DIRECTORY_DEPTH {
    return Err(CodecError::Message(format!(
      "UDF directory tree exceeds the maximum depth of {MAX_DIRECTORY_DEPTH}"
    )));
  }
  if !visited.insert((address.block, address.partition_map)) {
    return Err(CodecError::Message(format!(
      "UDF directory tree loops at block {}",
      address.block
    )));
  }

  let offset = resolve_address(image, partition, address)?;
  protected.push(ProtectedRange {
    offset: offset as u64,
    length: BLOCK_SIZE as u64,
    description: format!("UDF File Entry for directory '{path}'"),
  });
  let fe = descriptor_at(image, offset, TAG_FILE_ENTRY)?;
  let (directory_data, directory_extents) = read_file_data(image, partition, offset, fe)?;
  for extent in directory_extents {
    protected.push(ProtectedRange {
      offset: extent.offset,
      length: extent.length as u64,
      description: format!("UDF directory data for '{path}'"),
    });
  }
  let mut cursor = 0usize;

  while cursor + 38 <= directory_data.len() {
    let tag = u16_at(&directory_data, cursor)?;
    if tag == 0 {
      break;
    }
    if tag != TAG_FILE_IDENTIFIER {
      return Err(CodecError::Message(format!(
        "UDF directory '{}' contains tag {tag}, expected a File Identifier Descriptor",
        path
      )));
    }
    let record = &directory_data[cursor..];
    check_tag(record, TAG_FILE_IDENTIFIER)?;
    let characteristics = *record.get(18).ok_or_else(|| {
      CodecError::Message("UDF File Identifier Descriptor is truncated".to_owned())
    })?;
    let name_len = *record.get(19).ok_or_else(|| {
      CodecError::Message("UDF File Identifier Descriptor is truncated".to_owned())
    })? as usize;
    let icb = long_address(record, 20)?;
    let impl_len = u16_at(record, 36)? as usize;
    let name_start = 38usize
      .checked_add(impl_len)
      .ok_or_else(|| CodecError::Message("UDF file identifier length overflows".to_owned()))?;
    let name_end = name_start
      .checked_add(name_len)
      .ok_or_else(|| CodecError::Message("UDF file name length overflows".to_owned()))?;
    if name_end > record.len() {
      return Err(CodecError::Message(
        "UDF file identifier is truncated".to_owned(),
      ));
    }
    let record_size = align4(name_end);
    cursor = cursor
      .checked_add(record_size)
      .ok_or_else(|| CodecError::Message("UDF directory record length overflows".to_owned()))?;

    if characteristics & 0x04 != 0 || characteristics & 0x08 != 0 {
      continue;
    }
    let name = decode_name(&record[name_start..name_end])?;
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
      return Err(CodecError::Message(format!(
        "UDF file identifier '{name}' cannot be represented as an artifact path"
      )));
    }
    let child_path = if path.is_empty() {
      name.clone()
    } else {
      format!("{path}/{name}")
    };
    let child_offset = resolve_address(image, partition, icb)?;
    let child = descriptor_at(image, child_offset, TAG_FILE_ENTRY)?;
    let file_type = *child
      .get(27)
      .ok_or_else(|| CodecError::Message("UDF File Entry is truncated".to_owned()))?;

    if characteristics & 0x02 != 0 || file_type == 4 {
      walk_directory(
        image,
        partition,
        icb,
        &child_path,
        depth + 1,
        visited,
        files,
        protected,
      )?;
    } else {
      protected.push(ProtectedRange {
        offset: child_offset as u64,
        length: BLOCK_SIZE as u64,
        description: format!("UDF File Entry for '{child_path}'"),
      });
      let (data, extents) = read_file_data(image, partition, child_offset, child)?;
      files.push(UdfFile {
        path: child_path.clone(),
        aliases: vec![child_path.clone()],
        data,
        extents,
        file_entry_offsets: vec![child_offset as u64],
      });
    }
  }
  Ok(())
}

fn read_file_data(
  image: &[u8],
  partition: Partition,
  entry_offset: usize,
  entry: &[u8],
) -> Result<(Vec<u8>, Vec<DataExtent>), CodecError> {
  let file_length = usize::try_from(u64_at(entry, 56)?)
    .map_err(|_| CodecError::Message("UDF file length does not fit this machine".to_owned()))?;
  let extended_attributes = u32_at(entry, 168)? as usize;
  let allocation_length = u32_at(entry, 172)? as usize;
  let allocation_start = 176usize
    .checked_add(extended_attributes)
    .ok_or_else(|| CodecError::Message("UDF allocation descriptor offset overflows".to_owned()))?;
  let allocation_end = allocation_start
    .checked_add(allocation_length)
    .ok_or_else(|| CodecError::Message("UDF allocation descriptor length overflows".to_owned()))?;
  let allocation_data = entry.get(allocation_start..allocation_end).ok_or_else(|| {
    CodecError::Message("UDF allocation descriptors lie outside the File Entry".to_owned())
  })?;
  let flags = u16_at(entry, 34)? & 0x0007;
  if flags == 3 {
    return Err(CodecError::Message(
      "UDF inline file data is unsupported; this codec requires physical data extents".to_owned(),
    ));
  }
  if flags == 2 {
    return Err(CodecError::Message(
      "UDF extended allocation descriptors are unsupported".to_owned(),
    ));
  }
  let descriptor_size = if flags == 0 { 8 } else { 16 };
  if allocation_data.len() % descriptor_size != 0 {
    return Err(CodecError::Message(format!(
      "UDF allocation descriptor area has {} bytes, not a multiple of {descriptor_size}",
      allocation_data.len()
    )));
  }

  let mut allocation_capacity = 0usize;
  for descriptor in allocation_data.chunks_exact(descriptor_size) {
    let extent_length = (u32_at(descriptor, 0)? & 0x3fff_ffff) as usize;
    allocation_capacity = allocation_capacity
      .checked_add(extent_length)
      .ok_or_else(|| CodecError::Message("UDF allocation capacity overflows".to_owned()))?;
  }
  if file_length > allocation_capacity {
    return Err(CodecError::Message(format!(
      "UDF File Entry declares {file_length} bytes, but its allocation descriptors provide only {allocation_capacity}"
    )));
  }

  let mut result = Vec::new();
  let mut extents = Vec::new();
  let mut remaining = file_length;
  for descriptor in allocation_data.chunks_exact(descriptor_size) {
    let encoded_length = u32_at(descriptor, 0)?;
    let extent_type = encoded_length >> 30;
    let extent_length = (encoded_length & 0x3fff_ffff) as usize;
    if extent_type != 0 {
      return Err(CodecError::Message(format!(
        "UDF file uses allocation extent type {extent_type}; only recorded extents are supported"
      )));
    }
    let block_number = u32_at(descriptor, 4)?;
    let partition_map = if flags == 0 {
      0
    } else {
      u16_at(descriptor, 8)?
    };
    if partition_map != 0 {
      return Err(CodecError::Message(format!(
        "UDF file refers to partition map {partition_map}, but only map 0 is supported"
      )));
    }
    let blocks = extent_length.div_ceil(BLOCK_SIZE);
    let partition_end = u64::from(block_number) + blocks as u64;
    if partition_end > u64::from(partition.length) {
      return Err(CodecError::Message(format!(
        "UDF file allocation at partition block {block_number} spans {blocks} blocks, past the partition's {} blocks",
        partition.length
      )));
    }
    let count = remaining.min(extent_length);
    let absolute = partition
      .start
      .checked_add(block_number)
      .ok_or_else(|| CodecError::Message("UDF file extent location overflows".to_owned()))?;
    let offset = u64::from(absolute) * BLOCK_SIZE as u64;
    let start = usize::try_from(offset)
      .map_err(|_| CodecError::Message("UDF file offset does not fit this machine".to_owned()))?;
    let allocation_end = start
      .checked_add(extent_length)
      .ok_or_else(|| CodecError::Message("UDF file allocation range overflows".to_owned()))?;
    if allocation_end > image.len() {
      return Err(CodecError::Message(format!(
        "UDF file allocation at block {absolute} lies outside the {}-byte image",
        image.len()
      )));
    }
    let end = start
      .checked_add(count)
      .ok_or_else(|| CodecError::Message("UDF file data range overflows".to_owned()))?;
    let bytes = image.get(start..end).ok_or_else(|| {
      CodecError::Message(format!(
        "UDF file extent at block {absolute} lies outside the {}-byte image",
        image.len()
      ))
    })?;
    result.extend_from_slice(bytes);
    if extent_length > 0 {
      let extent_end = offset
        .checked_add(extent_length as u64)
        .ok_or_else(|| CodecError::Message("UDF file allocation range overflows".to_owned()))?;
      if extents.iter().any(|existing: &DataExtent| {
        existing.offset < extent_end
          && offset < existing.offset.saturating_add(existing.length as u64)
      }) {
        return Err(CodecError::Message(
          "UDF file contains overlapping allocation descriptors".to_owned(),
        ));
      }
      extents.push(DataExtent {
        offset,
        length: extent_length,
      });
    }
    remaining -= count;
  }

  debug_assert_eq!(result.len(), file_length);
  let _ = entry_offset;
  Ok((result, extents))
}

fn validate_udf_allocations(
  files: &[UdfFile],
  protected: &[ProtectedRange],
) -> Result<(), CodecError> {
  for file in files {
    for extent in &file.extents {
      let extent_range = ByteRange::new(extent.offset, extent.length as u64)
        .ok_or_else(|| CodecError::Message("UDF file allocation range overflows".to_owned()))?;
      for range in protected {
        let protected_range = ByteRange::new(range.offset, range.length)
          .ok_or_else(|| CodecError::Message("UDF protected range overflows".to_owned()))?;
        if extent_range.overlaps(protected_range) {
          return Err(CodecError::Message(format!(
            "UDF file '{}' allocation overlaps {} at byte {}",
            file.path, range.description, extent.offset
          )));
        }
      }
    }
  }

  for (index, file) in files.iter().enumerate() {
    for other in &files[index + 1..] {
      for extent in &file.extents {
        let extent_range = ByteRange::new(extent.offset, extent.length as u64)
          .ok_or_else(|| CodecError::Message("UDF file allocation range overflows".to_owned()))?;
        for other_extent in &other.extents {
          let other_range = ByteRange::new(other_extent.offset, other_extent.length as u64)
            .ok_or_else(|| CodecError::Message("UDF file allocation range overflows".to_owned()))?;
          if extent_range.overlaps(other_range) {
            return Err(CodecError::Message(format!(
              "UDF files '{}' and '{}' have overlapping allocations, but do not share a File Entry",
              file.path, other.path
            )));
          }
        }
      }
    }
  }
  Ok(())
}

fn image_range(image: &[u8], offset: u64, length: usize) -> Option<&[u8]> {
  let start = usize::try_from(offset).ok()?;
  let end = start.checked_add(length)?;
  image.get(start..end)
}

pub(super) fn validate_bridge_allocations(
  image: &[u8],
  udf: &UdfReading,
  iso: &iso_volume::Reading,
) -> Result<(), CodecError> {
  let mut iso_structures = vec![ProtectedRange {
    offset: 0,
    length: 16 * BLOCK_SIZE as u64,
    description: "the ISO 9660 System Area".to_owned(),
  }];

  let mut descriptor_block = 16u32;
  loop {
    let descriptor = iso_volume::block(image, descriptor_block)?;
    let offset = block_offset(u64::from(descriptor_block)).ok_or_else(|| {
      CodecError::Message("ISO 9660 descriptor block offset overflows".to_owned())
    })?;
    iso_structures.push(ProtectedRange {
      offset,
      length: BLOCK_SIZE as u64,
      description: format!("ISO 9660 volume descriptor at block {descriptor_block}"),
    });
    if descriptor[0] == iso_volume::DESCRIPTOR_TYPE_TERMINATOR {
      break;
    }
    descriptor_block = descriptor_block.checked_add(1).ok_or_else(|| {
      CodecError::Message("ISO 9660 descriptor block number overflows".to_owned())
    })?;
  }

  let path_table_bytes = iso.layout.path_table.size() as u64;
  for sector in [
    iso.layout.path_table.l_sector,
    iso.layout.path_table.m_sector,
    iso.layout.path_table.optional_l_sector,
    iso.layout.path_table.optional_m_sector,
  ]
  .into_iter()
  .filter(|sector| *sector != 0)
  {
    let offset = block_offset(u64::from(sector)).ok_or_else(|| {
      CodecError::Message("ISO 9660 path table block offset overflows".to_owned())
    })?;
    iso_structures.push(ProtectedRange {
      offset,
      length: path_table_bytes,
      description: format!("ISO 9660 path table at block {sector}"),
    });
  }
  for directory in &iso.layout.directories {
    let offset = block_offset(u64::from(directory.extent))
      .ok_or_else(|| CodecError::Message("ISO 9660 directory block offset overflows".to_owned()))?;
    iso_structures.push(ProtectedRange {
      offset,
      length: directory.length.div_ceil(BLOCK_SIZE as u32) as u64 * BLOCK_SIZE as u64,
      description: format!("ISO 9660 directory '{}'", directory.path),
    });
  }

  for file in &udf.files {
    for extent in &file.extents {
      let extent_range = ByteRange::new(extent.offset, extent.length as u64)
        .ok_or_else(|| CodecError::Message("UDF file allocation range overflows".to_owned()))?;
      for protected in &iso_structures {
        let protected_range = ByteRange::new(protected.offset, protected.length)
          .ok_or_else(|| CodecError::Message("ISO 9660 protected range overflows".to_owned()))?;
        if extent_range.overlaps(protected_range) {
          return Err(CodecError::Message(format!(
            "UDF file '{}' allocation overlaps {} at byte {}",
            file.path, protected.description, extent.offset
          )));
        }
      }

      for iso_file in &iso.files {
        let iso_offset = block_offset(u64::from(iso_file.extent))
          .ok_or_else(|| CodecError::Message("ISO 9660 file block offset overflows".to_owned()))?;
        let iso_length = iso_file.length;
        let iso_allocation = iso_length.div_ceil(BLOCK_SIZE) as u64 * BLOCK_SIZE as u64;
        let iso_range = ByteRange::new(iso_offset, iso_allocation).ok_or_else(|| {
          CodecError::Message("ISO 9660 file allocation range overflows".to_owned())
        })?;
        if !extent_range.overlaps(iso_range) {
          continue;
        }

        let shared = file.extents.len() == 1
          && extent.offset == iso_offset
          && file.data.len() == iso_length
          && image_range(image, iso_offset, iso_length)
            .is_some_and(|bytes| file.data.as_slice() == bytes);
        if !shared {
          return Err(CodecError::Message(format!(
            "UDF file '{}' allocation overlaps ISO 9660 file '{}' without matching its physical extent and length",
            file.path, iso_file.name
          )));
        }
      }
    }
  }
  Ok(())
}

pub(super) fn descriptor_at(
  image: &[u8],
  offset: usize,
  expected_tag: u16,
) -> Result<&[u8], CodecError> {
  let block_number = u32::try_from(offset / BLOCK_SIZE)
    .map_err(|_| CodecError::Message("UDF descriptor location does not fit".to_owned()))?;
  let descriptor = image
    .get(offset..)
    .and_then(|bytes| bytes.get(..BLOCK_SIZE))
    .ok_or_else(|| CodecError::Message(format!("UDF descriptor at byte {offset} is truncated")))?;
  check_tag(descriptor, expected_tag)?;
  let _ = block_number;
  Ok(descriptor)
}

fn resolve_address(
  image: &[u8],
  partition: Partition,
  address: LongAddress,
) -> Result<usize, CodecError> {
  if address.partition_map != 0 {
    return Err(CodecError::Message(format!(
      "UDF address refers to partition map {}, but only map 0 is supported",
      address.partition_map
    )));
  }
  if address.block >= partition.length {
    return Err(CodecError::Message(format!(
      "UDF block {} lies outside partition {} of {} blocks",
      address.block, partition.number, partition.length
    )));
  }
  let absolute = partition.start.checked_add(address.block).ok_or_else(|| {
    CodecError::Message("UDF address overflows the partition location".to_owned())
  })?;
  let offset = usize::try_from(absolute)
    .ok()
    .and_then(|block| block.checked_mul(BLOCK_SIZE))
    .ok_or_else(|| CodecError::Message("UDF address does not fit this machine".to_owned()))?;
  if offset >= image.len() {
    return Err(CodecError::Message(format!(
      "UDF block {absolute} lies outside the image"
    )));
  }
  Ok(offset)
}

fn block(image: &[u8], block: u32) -> Result<&[u8], CodecError> {
  let offset = usize::try_from(block)
    .ok()
    .and_then(|block| block.checked_mul(BLOCK_SIZE))
    .ok_or_else(|| CodecError::Message("UDF block address overflows".to_owned()))?;
  image
    .get(offset..)
    .and_then(|bytes| bytes.get(..BLOCK_SIZE))
    .ok_or_else(|| CodecError::Message(format!("UDF block {block} lies outside the image")))
}

pub(super) fn check_tag(descriptor: &[u8], expected: u16) -> Result<(), CodecError> {
  let found = tag_id(descriptor)?;
  if found != expected {
    return Err(CodecError::Message(format!(
      "Expected UDF descriptor tag {expected}, found {found}"
    )));
  }
  if descriptor.len() < 16 {
    return Err(CodecError::Message(
      "UDF descriptor tag is truncated".to_owned(),
    ));
  }
  let checksum = descriptor[..4]
    .iter()
    .chain(descriptor[5..16].iter())
    .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
  if checksum != descriptor[4] {
    return Err(CodecError::Message(format!(
      "UDF descriptor tag {expected} has checksum {:#04x}, expected {checksum:#04x}",
      descriptor[4]
    )));
  }
  let crc_len = u16_at(descriptor, 10)? as usize;
  if crc_len > descriptor.len() - 16 {
    return Err(CodecError::Message(format!(
      "UDF descriptor tag {expected} declares CRC length {crc_len}, exceeding its available bytes"
    )));
  }
  let expected_crc = u16_at(descriptor, 8)?;
  if crc_len > 0 {
    let actual_crc = udf_crc16(&descriptor[16..16 + crc_len]);
    if actual_crc != expected_crc {
      return Err(CodecError::Message(format!(
        "UDF descriptor tag {expected} has CRC {expected_crc:#06x}, expected {actual_crc:#06x}"
      )));
    }
  }
  Ok(())
}

pub(super) fn udf_crc16(bytes: &[u8]) -> u16 {
  bytes.iter().fold(0u16, |mut crc, byte| {
    crc ^= u16::from(*byte) << 8;
    for _ in 0..8 {
      crc = if crc & 0x8000 != 0 {
        (crc << 1) ^ 0x1021
      } else {
        crc << 1
      };
    }
    crc
  })
}

fn tag_id(data: &[u8]) -> Result<u16, CodecError> {
  u16_at(data, 0)
}

fn long_address(data: &[u8], offset: usize) -> Result<LongAddress, CodecError> {
  Ok(LongAddress {
    block: u32_at(data, offset + 4)?,
    partition_map: u16_at(data, offset + 8)?,
  })
}

fn extent_ad(data: &[u8], offset: usize) -> Result<(u32, u32), CodecError> {
  Ok((u32_at(data, offset)?, u32_at(data, offset + 4)?))
}

pub(super) fn u16_at(data: &[u8], offset: usize) -> Result<u16, CodecError> {
  let bytes = data
    .get(offset..offset + 2)
    .ok_or_else(|| CodecError::Message(format!("UDF field at byte {offset} is truncated")))?;
  Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

pub(super) fn u32_at(data: &[u8], offset: usize) -> Result<u32, CodecError> {
  let bytes = data
    .get(offset..offset + 4)
    .ok_or_else(|| CodecError::Message(format!("UDF field at byte {offset} is truncated")))?;
  Ok(u32::from_le_bytes([
    bytes[0],
    bytes[1],
    bytes[2],
    bytes[3],
  ]))
}

pub(super) fn u64_at(data: &[u8], offset: usize) -> Result<u64, CodecError> {
  let bytes = data
    .get(offset..offset + 8)
    .ok_or_else(|| CodecError::Message(format!("UDF field at byte {offset} is truncated")))?;
  Ok(u64::from_le_bytes(
    bytes.try_into().expect("slice length was checked"),
  ))
}

pub(super) fn align4(value: usize) -> usize {
  value.saturating_add(3) & !3
}

fn decode_name(bytes: &[u8]) -> Result<String, CodecError> {
  let (&compression, encoded) = bytes
    .split_first()
    .ok_or_else(|| CodecError::Message("UDF file identifier has no compression byte".to_owned()))?;
  match compression {
    8 => encoded
      .iter()
      .map(|byte| char::from_u32(u32::from(*byte)))
      .collect::<Option<String>>()
      .ok_or_else(|| CodecError::Message("UDF file name has invalid 8-bit characters".to_owned())),
    16 if encoded.len().is_multiple_of(2) => {
      let units = encoded
        .as_chunks::<2>()
        .0
        .iter()
        .map(|unit| u16::from_be_bytes(*unit))
        .collect::<Vec<_>>();
      String::from_utf16(&units)
        .map_err(|_| CodecError::Message("UDF file name has invalid UTF-16".to_owned()))
    }
    _ => Err(CodecError::Message(format!(
      "UDF file name compression id {compression} is unsupported"
    ))),
  }
}
