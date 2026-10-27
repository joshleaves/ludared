//! UDF file writes and preservation of bytes outside logical file contents.
use std::collections::BTreeMap;

use super::metadata::DataExtent;
use super::metadata::FileLayout;
use super::metadata::OpaqueRun;
use super::volume::TAG_FILE_ENTRY;
use super::volume::UdfReading;
use super::volume::check_tag;
use super::volume::u16_at;
use super::volume::udf_crc16;
use crate::codecs::DecodedArtifact;
use crate::codecs::cdrom::common_disk::BLOCK_SIZE;
use crate::codecs::cdrom::common_disk::Hex;
use crate::codecs::errors::CodecError;

pub(super) fn validate_rebuilt_udf_files(
  metadata_files: &[FileLayout],
  supplied: &BTreeMap<&str, &DecodedArtifact>,
  rebuilt: &UdfReading,
) -> Result<(), CodecError> {
  for layout in metadata_files
    .iter()
    .filter(|file| !file.file_entry_offsets.is_empty())
  {
    let expected = supplied[layout.artifact.as_str()];
    let actual = rebuilt
      .files
      .iter()
      .find(|file| file.path == layout.artifact || file.aliases.contains(&layout.artifact))
      .ok_or_else(|| {
        CodecError::Message(format!("Rebuilt UDF image lost file '{}'", layout.artifact))
      })?;
    if actual.data != expected.data || actual.extents != layout.extents {
      return Err(CodecError::Message(format!(
        "Rebuilt UDF file '{}' does not match its artifact bytes and recorded allocation",
        layout.artifact
      )));
    }
  }
  Ok(())
}

pub(super) fn opaque_runs(
  image: &[u8],
  files: &[FileLayout],
) -> Result<Vec<OpaqueRun>, CodecError> {
  let mut ranges = Vec::new();
  for file in files {
    let mut remaining = file.length;
    for extent in &file.extents {
      let length = extent.length.min(remaining);
      if length > 0 {
        let end = extent
          .offset
          .checked_add(length as u64)
          .ok_or_else(|| CodecError::Message("UDF file range overflows".to_owned()))?;
        ranges.push((extent.offset, end));
        remaining -= length;
      }
    }
    if remaining != 0 {
      return Err(CodecError::Message(format!(
        "UDF file '{}' has {remaining} bytes without allocation extents",
        file.artifact
      )));
    }
  }
  ranges.sort_unstable();

  let mut merged: Vec<(u64, u64)> = Vec::new();
  for (start, end) in ranges {
    if let Some((_, previous_end)) = merged.last_mut()
      && start <= *previous_end
    {
      *previous_end = (*previous_end).max(end);
    } else {
      merged.push((start, end));
    }
  }

  let mut opaque = Vec::new();
  let mut cursor = 0u64;
  for (start, end) in merged {
    if start > cursor {
      let start_usize = usize::try_from(cursor)
        .map_err(|_| CodecError::Message("UDF opaque offset does not fit".to_owned()))?;
      let end_usize = usize::try_from(start)
        .map_err(|_| CodecError::Message("UDF opaque offset does not fit".to_owned()))?;
      opaque.push(OpaqueRun {
        offset: cursor,
        data: Hex(image[start_usize..end_usize].to_vec()),
      });
    }
    cursor = cursor.max(end);
  }
  if cursor < image.len() as u64 {
    let start = usize::try_from(cursor)
      .map_err(|_| CodecError::Message("UDF opaque offset does not fit".to_owned()))?;
    opaque.push(OpaqueRun {
      offset: cursor,
      data: Hex(image[start..].to_vec()),
    });
  }
  Ok(opaque)
}

pub(super) fn remove_opaque_range(
  opaque: &mut Vec<crate::codecs::cdrom::iso9660::metadata::Opaque>,
  extent: &DataExtent,
) -> Result<(), CodecError> {
  let removed_start = extent.offset;
  let removed_end = removed_start
    .checked_add(extent.length as u64)
    .ok_or_else(|| CodecError::Message("UDF file extent range overflows".to_owned()))?;
  let mut remaining = Vec::with_capacity(opaque.len() + 1);

  for run in opaque.drain(..) {
    let run_start = run.offset;
    let run_end = run_start
      .checked_add(run.data.len() as u64)
      .ok_or_else(|| CodecError::Message("ISO opaque range overflows".to_owned()))?;
    if removed_end <= run_start || removed_start >= run_end {
      remaining.push(run);
      continue;
    }

    if removed_start > run_start {
      let keep = usize::try_from(removed_start - run_start).map_err(|_| {
        CodecError::Message("ISO opaque range does not fit this machine".to_owned())
      })?;
      remaining.push(crate::codecs::cdrom::iso9660::metadata::Opaque {
        offset: run_start,
        data: Hex(run.data.as_bytes()[..keep].to_vec()),
      });
    }
    if removed_end < run_end {
      let skip = usize::try_from(removed_end - run_start).map_err(|_| {
        CodecError::Message("ISO opaque range does not fit this machine".to_owned())
      })?;
      remaining.push(crate::codecs::cdrom::iso9660::metadata::Opaque {
        offset: removed_end,
        data: Hex(run.data.as_bytes()[skip..].to_vec()),
      });
    }
  }
  *opaque = remaining;
  Ok(())
}

pub(super) fn write_file_extents(
  image: &mut [u8],
  file: &FileLayout,
  data: &[u8],
) -> Result<(), CodecError> {
  let mut cursor = 0usize;
  for extent in &file.extents {
    let count = extent.length.min(data.len().saturating_sub(cursor));
    let start = usize::try_from(extent.offset)
      .map_err(|_| CodecError::Message("UDF file offset does not fit this machine".to_owned()))?;
    let end = start
      .checked_add(count)
      .ok_or_else(|| CodecError::Message("UDF file range overflows the image".to_owned()))?;
    image
      .get_mut(start..end)
      .ok_or_else(|| CodecError::Message("UDF file range lies outside the image".to_owned()))?
      .copy_from_slice(&data[cursor..cursor + count]);
    cursor += count;
  }
  if cursor != data.len() {
    return Err(CodecError::Message(format!(
      "UDF file '{}' has {} bytes, but its recorded extents can hold {cursor}",
      file.artifact,
      data.len()
    )));
  }
  Ok(())
}

pub(super) fn update_file_entries(
  image: &mut [u8],
  file: &FileLayout,
  new_length: usize,
) -> Result<(), CodecError> {
  let new_length = u64::try_from(new_length)
    .map_err(|_| CodecError::Message("UDF file length does not fit in 64 bits".to_owned()))?;
  for &descriptor_offset in &file.file_entry_offsets {
    let start = usize::try_from(descriptor_offset)
      .map_err(|_| CodecError::Message("UDF File Entry offset does not fit".to_owned()))?;
    let end = start
      .checked_add(BLOCK_SIZE)
      .ok_or_else(|| CodecError::Message("UDF File Entry range overflows".to_owned()))?;
    let descriptor = image
      .get_mut(start..end)
      .ok_or_else(|| CodecError::Message("UDF File Entry lies outside the image".to_owned()))?;
    check_tag(descriptor, TAG_FILE_ENTRY)?;
    descriptor[56..64].copy_from_slice(&new_length.to_le_bytes());

    let crc_length = usize::from(u16_at(descriptor, 10)?);
    let crc_end = 16usize
      .checked_add(crc_length)
      .ok_or_else(|| CodecError::Message("UDF File Entry CRC range overflows".to_owned()))?;
    let crc_data = descriptor.get(16..crc_end).ok_or_else(|| {
      CodecError::Message("UDF File Entry CRC range lies outside the descriptor".to_owned())
    })?;
    let crc = udf_crc16(crc_data);
    descriptor[8..10].copy_from_slice(&crc.to_le_bytes());
    descriptor[4] = descriptor[..4]
      .iter()
      .chain(descriptor[5..16].iter())
      .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
  }
  Ok(())
}
