//! UDF metadata and validation for the rebuild artifact.
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::codecs::cdrom::common::BLOCK_SIZE;
use crate::codecs::cdrom::common::Hex;
use crate::codecs::errors::CodecError;

pub(super) const METADATA_NAME: &str = "UDF.json";

pub(super) fn validate_metadata(metadata: &Metadata) -> Result<(), CodecError> {
  if metadata.image_len == 0 || !metadata.image_len.is_multiple_of(BLOCK_SIZE) {
    return Err(CodecError::Message(format!(
      "UDF metadata declares an invalid image size of {} bytes",
      metadata.image_len
    )));
  }
  if metadata.bridge.is_some() && !metadata.opaque.is_empty() {
    return Err(CodecError::Message(
      "UDF Bridge metadata must keep opaque image bytes in its ISO 9660 metadata artifact"
        .to_owned(),
    ));
  }

  let mut artifact_names = BTreeSet::new();
  let mut aliases = BTreeMap::new();
  let mut file_entries = BTreeMap::new();
  let mut allocations = Vec::new();
  let mut logical_data = Vec::new();
  for file in &metadata.files {
    validate_artifact_path(&file.artifact)?;
    if file.artifact == METADATA_NAME {
      return Err(CodecError::Message(format!(
        "UDF file artifact '{}' conflicts with the metadata artifact",
        file.artifact
      )));
    }
    if !artifact_names.insert(file.artifact.as_str()) {
      return Err(CodecError::Message(format!(
        "UDF metadata describes duplicate artifact '{}'",
        file.artifact
      )));
    }

    if !file.file_entry_offsets.is_empty() {
      let capacity = file
        .extents
        .iter()
        .try_fold(0usize, |sum, extent| sum.checked_add(extent.length));
      let Some(capacity) = capacity else {
        return Err(CodecError::Message(format!(
          "UDF file '{}' allocation capacity overflows",
          file.artifact
        )));
      };
      if file.length > capacity {
        return Err(CodecError::Message(format!(
          "UDF metadata file '{}' declares {} bytes, but its allocations hold {capacity}",
          file.artifact, file.length
        )));
      }
      for extent in &file.extents {
        let end = extent
          .offset
          .checked_add(extent.length as u64)
          .ok_or_else(|| CodecError::Message("UDF file allocation range overflows".to_owned()))?;
        if end > metadata.image_len as u64 {
          return Err(CodecError::Message(format!(
            "UDF file '{}' allocation ends at byte {end}, past the {}-byte image",
            file.artifact, metadata.image_len
          )));
        }
        allocations.push((extent.offset, end, file.artifact.as_str()));
      }

      let mut remaining = file.length;
      for extent in &file.extents {
        let length = remaining.min(extent.length);
        if length > 0 {
          let end = extent
            .offset
            .checked_add(length as u64)
            .ok_or_else(|| CodecError::Message("UDF file data range overflows".to_owned()))?;
          logical_data.push((extent.offset, end, file.artifact.as_str()));
          remaining -= length;
        }
      }
      if remaining != 0 {
        return Err(CodecError::Message(format!(
          "UDF file '{}' has {remaining} bytes without allocated extents",
          file.artifact
        )));
      }
    } else if metadata.bridge.is_none() {
      return Err(CodecError::Message(format!(
        "Standalone UDF artifact '{}' has no File Entry references",
        file.artifact
      )));
    }

    for &offset in &file.file_entry_offsets {
      if !offset.is_multiple_of(BLOCK_SIZE as u64)
        || offset
          .checked_add(BLOCK_SIZE as u64)
          .is_none_or(|end| end > metadata.image_len as u64)
      {
        return Err(CodecError::Message(format!(
          "UDF File Entry offset {offset} for '{}' lies outside the image or is unaligned",
          file.artifact
        )));
      }
      if let Some(previous) = file_entries.insert(offset, file.artifact.as_str())
        && previous != file.artifact
      {
        return Err(CodecError::Message(format!(
          "UDF File Entry at byte {offset} is assigned to both '{previous}' and '{}'",
          file.artifact
        )));
      }
    }
    for alias in &file.aliases {
      validate_artifact_path(alias)?;
      if let Some(previous) = aliases.insert(alias.as_str(), file.artifact.as_str())
        && previous != file.artifact
      {
        return Err(CodecError::Message(format!(
          "UDF alias '{alias}' is assigned to both '{previous}' and '{}'",
          file.artifact
        )));
      }
    }
  }

  let names: Vec<_> = artifact_names.iter().copied().collect();
  for (index, name) in names.iter().enumerate() {
    if names[index + 1..].iter().any(|other| {
      other
        .strip_prefix(name)
        .is_some_and(|tail| tail.starts_with('/'))
    }) {
      return Err(CodecError::Message(format!(
        "UDF artifact path '{name}' conflicts with a nested artifact path"
      )));
    }
  }

  allocations.sort_unstable_by_key(|range| range.0);
  for pair in allocations.windows(2) {
    if pair[0].1 > pair[1].0 {
      return Err(CodecError::Message(format!(
        "UDF artifacts '{}' and '{}' have overlapping recorded allocations",
        pair[0].2, pair[1].2
      )));
    }
  }

  let mut opaque_ranges = Vec::with_capacity(metadata.opaque.len());
  for run in &metadata.opaque {
    let end = run
      .offset
      .checked_add(run.data.len() as u64)
      .ok_or_else(|| CodecError::Message("UDF opaque range overflows".to_owned()))?;
    if end > metadata.image_len as u64 {
      return Err(CodecError::Message(format!(
        "UDF opaque range ends at byte {end}, past the {}-byte image",
        metadata.image_len
      )));
    }
    opaque_ranges.push((run.offset, end));
  }
  opaque_ranges.sort_unstable();
  for pair in opaque_ranges.windows(2) {
    if pair[0].1 > pair[1].0 {
      return Err(CodecError::Message(
        "UDF metadata contains overlapping opaque byte ranges".to_owned(),
      ));
    }
  }
  for (start, end, name) in &logical_data {
    if opaque_ranges
      .iter()
      .any(|(opaque_start, opaque_end)| start < opaque_end && opaque_start < end)
    {
      return Err(CodecError::Message(format!(
        "UDF file data for '{name}' is duplicated in opaque metadata"
      )));
    }
  }

  if metadata.bridge.is_none() {
    let mut ranges = opaque_ranges
      .into_iter()
      .map(|(start, end)| (start, end, "opaque"))
      .chain(logical_data)
      .collect::<Vec<_>>();
    ranges.sort_unstable_by_key(|range| range.0);
    let mut cursor = 0u64;
    for (start, end, name) in ranges {
      if start != cursor {
        return Err(CodecError::Message(format!(
          "Standalone UDF metadata has a gap or overlap before {name} at byte {start}"
        )));
      }
      cursor = end;
    }
    if cursor != metadata.image_len as u64 {
      return Err(CodecError::Message(format!(
        "Standalone UDF metadata covers {cursor} bytes, but the image has {}",
        metadata.image_len
      )));
    }
  }

  if let Some(bridge) = &metadata.bridge {
    for iso_file in &bridge.iso_files {
      let Some(file) = metadata
        .files
        .iter()
        .find(|file| file.artifact == iso_file.artifact)
      else {
        return Err(CodecError::Message(format!(
          "ISO 9660 file '{}' references missing artifact '{}'",
          iso_file.name, iso_file.artifact
        )));
      };
      if file.iso_name.as_deref() != Some(iso_file.name.as_str()) {
        return Err(CodecError::Message(format!(
          "UDF artifact '{}' has an inconsistent ISO 9660 mapping",
          iso_file.artifact
        )));
      }
    }
  } else if metadata.files.iter().any(|file| file.iso_name.is_some()) {
    return Err(CodecError::Message(
      "Standalone UDF metadata contains ISO 9660 file mappings".to_owned(),
    ));
  }
  Ok(())
}

pub(super) fn validate_artifact_path(path: &str) -> Result<(), CodecError> {
  if path.is_empty()
    || path.starts_with('/')
    || path.contains('\\')
    || path
      .split('/')
      .any(|component| component.is_empty() || matches!(component, "." | ".."))
  {
    return Err(CodecError::Message(format!(
      "UDF metadata contains invalid artifact path '{path}'"
    )));
  }
  Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Metadata {
  pub(super) version: u8,
  pub(super) image_len: usize,
  pub(super) revision: u16,
  pub(super) bridge: Option<BridgeMetadata>,
  pub(super) files: Vec<FileLayout>,
  pub(super) opaque: Vec<OpaqueRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct BridgeMetadata {
  pub(super) iso_metadata: Vec<u8>,
  pub(super) iso_files: Vec<IsoFileLayout>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct IsoFileLayout {
  pub(super) name: String,
  pub(super) extent: u64,
  pub(super) length: usize,
  pub(super) artifact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FileLayout {
  pub(super) artifact: String,
  pub(super) aliases: Vec<String>,
  pub(super) length: usize,
  pub(super) extents: Vec<DataExtent>,
  pub(super) file_entry_offsets: Vec<u64>,
  pub(super) iso_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct DataExtent {
  pub(super) offset: u64,
  pub(super) length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OpaqueRun {
  pub(super) offset: u64,
  pub(super) data: Hex,
}
