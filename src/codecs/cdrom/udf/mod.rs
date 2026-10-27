//! UDF filesystem extraction and reconstruction, including UDF Bridge images.
use log::*;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::cdrom::common_disk::BLOCK_SIZE;
use crate::codecs::cdrom::common_disk::Hex;
use crate::codecs::cdrom::common_disk::block_offset;
use crate::codecs::cdrom::iso9660::Iso9660;
use crate::codecs::cdrom::iso9660::Placement;
use crate::codecs::cdrom::iso9660::metadata::IsoMetadata;
use crate::codecs::cdrom::iso9660::volume as iso_volume;
use crate::codecs::errors::CodecError;
use metadata::BridgeMetadata;
use metadata::DataExtent;
use metadata::FileLayout;
use metadata::IsoFileLayout;
use metadata::METADATA_NAME;
use metadata::Metadata;
use metadata::OpaqueRun;
use metadata::validate_metadata;
use rebuild::opaque_runs;
use rebuild::remove_opaque_range;
use rebuild::update_file_entries;
use rebuild::validate_rebuilt_udf_files;
use rebuild::write_file_extents;
use volume::has_udf_vrs;
use volume::read_udf;
use volume::valid_anchor;
use volume::validate_bridge_allocations;

mod metadata;
mod rebuild;
mod volume;

#[cfg(test)]
mod tests;

const CODEC_ID: &str = "std/cdrom/udf";
const CODEC_NAME: &str = "UDF filesystem image";
const CODEC_DESC: &str = r#"UDF filesystem image codec

Extracts a UDF file tree into file artifacts and a UDF.json metadata artifact.
For UDF Bridge images it also reads the parallel ISO 9660 tree, maps files by
their physical extents, and produces only one artifact for shared file data.
The ISO 9660 codec rebuilds bridge images so both trees and all opaque bytes
remain in their original layout.

Unchanged images are rebuilt byte for byte. Files may shrink or grow within
their existing allocated blocks; matching UDF and ISO 9660 lengths are updated
together. Moving a file or allocating additional blocks is refused until the
codec can safely update UDF free-space metadata. Standalone UDF images with
allocation structures outside the supported physical-partition subset are
refused explicitly.

Supported UDF revisions are 1.02 through 1.50. The revision recorded by the
image is preserved. This codec currently reads one physical partition map,
File Entries, and short or long recorded allocation descriptors. Allocation
extent continuation descriptors, inline file data, and virtual or sparable
partitions are not supported. Files may use multiple recorded extents.

Arguments (JSON):
  placement (default: "relocate")
    For UDF Bridge images, forwards the ISO 9660 placement policy: "relocate",
    "shift", or "preserve". Current UDF safety checks refuse every change that
    needs a file to move or needs additional UDF allocation, regardless of this
    setting. Supported size changes stay within existing allocations, so this
    setting has no effect on them. Standalone UDF images do not use it.

Example:
  {"placement":"preserve"}
"#;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UdfArgs {
  #[serde(default)]
  placement: Placement,
}

impl LudaredCodecArgs for UdfArgs {}

pub(crate) struct Udf;

impl Codec for Udf {
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
    CodecKind::Rebuild
  }

  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    if has_udf_vrs(data) && valid_anchor(data) {
      Ok(CodecHandlingConfidence::Certain)
    } else {
      Ok(CodecHandlingConfidence::No)
    }
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("udf".to_owned())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    let UdfArgs { placement: _ } = UdfArgs::from_args(args)?;
    trace!("Decoding UDF image");

    let udf = read_udf(data)?;
    let iso_reading = if Iso9660.can_handle(data)? == CodecHandlingConfidence::Certain {
      Some(iso_volume::read_layout(
        data,
        crate::codecs::cdrom::iso9660::OutOfBoundsEntries::Preserve,
      )?)
    } else {
      None
    };

    let mut bridge = None;
    let mut iso_file_layouts = Vec::new();
    let mut udf_files = Vec::new();

    if let Some(reading) = iso_reading.as_ref() {
      validate_bridge_allocations(data, &udf, reading)?;
      let mut saved_iso_metadata = IsoMetadata {
        volume_identifier: reading.volume_identifier.clone(),
        file_order: reading.files.iter().map(|file| file.name.clone()).collect(),
        layout: reading.layout.clone(),
      };

      for iso_file in &reading.files {
        let offset = block_offset(u64::from(iso_file.extent)).ok_or_else(|| {
          CodecError::Message("ISO 9660 file extent overflows its byte offset".to_owned())
        })?;
        let iso_data = image_bytes_at(data, offset, iso_file.length, &iso_file.name)?;
        for udf_file in &udf.files {
          for extent in &udf_file.extents {
            let udf_end = extent.offset + extent.length.min(udf_file.data.len()) as u64;
            let iso_end = offset + iso_file.length as u64;
            if extent.offset < iso_end
              && offset < udf_end
              && (extent.offset != offset || udf_file.data.len() != iso_file.length)
            {
              return Err(CodecError::Message(format!(
                "UDF file '{}' and ISO 9660 file '{}' overlap at different lengths or offsets; their shared data cannot be represented by one artifact",
                udf_file.path, iso_file.name
              )));
            }
          }
        }
        let matching = udf.files.iter().find(|file| {
          file.extents.len() == 1
            && file.extents[0].offset == offset
            && file.data.len() == iso_file.length
        });

        let artifact_name = if let Some(file) = matching {
          if file.data.as_slice() != iso_data {
            return Err(CodecError::Message(format!(
              "UDF file '{}' and ISO 9660 file '{}' share an extent but contain different bytes",
              file.path, iso_file.name
            )));
          }
          file.path.clone()
        } else {
          format!("ISO9660/{}", iso_file.name)
        };

        iso_file_layouts.push(IsoFileLayout {
          name: iso_file.name.clone(),
          extent: offset,
          length: iso_file.length,
          artifact: artifact_name,
        });
      }

      for file in &udf.files {
        let iso_name = reading
          .files
          .iter()
          .find(|iso_file| {
            file.extents.len() == 1
              && block_offset(u64::from(iso_file.extent))
                .is_some_and(|offset| file.extents[0].offset == offset)
              && file.data.len() == iso_file.length
              && block_offset(u64::from(iso_file.extent)).is_some_and(|offset| {
                image_bytes_at(data, offset, iso_file.length, &iso_file.name)
                  .is_ok_and(|bytes| file.data.as_slice() == bytes)
              })
          })
          .map(|iso_file| iso_file.name.clone());

        udf_files.push(FileLayout {
          artifact: file.path.clone(),
          aliases: file.aliases.clone(),
          length: file.data.len(),
          extents: file.extents.clone(),
          file_entry_offsets: file.file_entry_offsets.clone(),
          iso_name,
        });
      }

      for file in udf_files.iter().filter(|file| file.iso_name.is_none()) {
        let mut remaining = file.length;
        for extent in &file.extents {
          let length = extent.length.min(remaining);
          remove_opaque_range(
            &mut saved_iso_metadata.layout.opaque,
            &DataExtent {
              offset: extent.offset,
              length,
            },
          )?;
          remaining -= length;
        }
        if remaining != 0 {
          return Err(CodecError::Message(format!(
            "UDF-only file '{}' has {} bytes without corresponding allocation extents",
            file.artifact, remaining
          )));
        }
      }

      bridge = Some(BridgeMetadata {
        iso_metadata: saved_iso_metadata.to_json()?,
        iso_files: iso_file_layouts.clone(),
      });

      // A bridge's ISO metadata captures every byte not represented by its
      // ISO files, including UDF descriptors and UDF-only data. Its rebuild
      // therefore remains the single image-layout writer.

      for iso_file in &iso_file_layouts {
        if iso_file.artifact.starts_with("ISO9660/") {
          udf_files.push(FileLayout {
            artifact: iso_file.artifact.clone(),
            aliases: Vec::new(),
            length: iso_file.length,
            extents: vec![DataExtent {
              offset: iso_file.extent,
              length: iso_file.length.div_ceil(BLOCK_SIZE) * BLOCK_SIZE,
            }],
            file_entry_offsets: Vec::new(),
            iso_name: Some(iso_file.name.clone()),
          });
        }
      }
    } else {
      udf_files = udf
        .files
        .iter()
        .map(|file| FileLayout {
          artifact: file.path.clone(),
          aliases: file.aliases.clone(),
          length: file.data.len(),
          extents: file.extents.clone(),
          file_entry_offsets: file.file_entry_offsets.clone(),
          iso_name: None,
        })
        .collect();
    }

    let mut artifact_names = BTreeSet::new();
    for file in &udf_files {
      if !artifact_names.insert(file.artifact.as_str()) {
        return Err(CodecError::Message(format!(
          "UDF image contains multiple files under artifact path '{}'",
          file.artifact
        )));
      }
    }

    let mut artifacts = vec![DecodedArtifact {
      name: METADATA_NAME.to_owned(),
      data: {
        let is_bridge = bridge.is_some();
        serde_json::to_vec_pretty(&Metadata {
          version: 1,
          image_len: data.len(),
          revision: udf.revision,
          bridge: bridge.clone(),
          files: udf_files.clone(),
          opaque: if is_bridge {
            Vec::new()
          } else if udf_files.iter().all(|file| file.extents.is_empty()) {
            vec![OpaqueRun {
              offset: 0,
              data: Hex(data.to_vec()),
            }]
          } else {
            opaque_runs(data, &udf_files)?
          },
        })
        .map_err(|err| CodecError::Message(format!("Could not write UDF metadata: {err}")))?
      },
    }];

    let mut emitted = BTreeSet::new();
    let mut ordered_files: Vec<_> = udf.files.iter().collect();
    ordered_files.sort_by_key(|file| {
      file
        .extents
        .first()
        .map_or(u64::MAX, |extent| extent.offset)
    });

    for file in ordered_files {
      let name = file.path.clone();
      if emitted.insert(name.clone()) {
        artifacts.push(DecodedArtifact {
          name,
          data: file.data.clone(),
        });
      }
    }

    if let Some(bridge) = &bridge {
      for file in &bridge.iso_files {
        if file.artifact.starts_with("ISO9660/") && emitted.insert(file.artifact.clone()) {
          artifacts.push(DecodedArtifact {
            name: file.artifact.clone(),
            data: image_bytes_at(data, file.extent, file.length, &file.name)?.to_vec(),
          });
        }
      }
    }

    Ok(artifacts)
  }

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    let UdfArgs { placement } = UdfArgs::from_args(args)?;
    trace!("Encoding UDF image");

    let (metadata_artifact, files) = artifacts.split_first().ok_or_else(|| {
      CodecError::Message("Expected a UDF metadata artifact, found no artifacts".to_owned())
    })?;
    if metadata_artifact.name != METADATA_NAME {
      return Err(CodecError::Message(format!(
        "Expected the first artifact to be '{METADATA_NAME}', found '{}'",
        metadata_artifact.name
      )));
    }

    let metadata: Metadata = serde_json::from_slice(&metadata_artifact.data)
      .map_err(|err| CodecError::Message(format!("Could not read UDF metadata: {err}")))?;
    if metadata.version != 1 {
      return Err(CodecError::Message(format!(
        "Unsupported UDF metadata version {}",
        metadata.version
      )));
    }
    if !(0x0102..=0x0150).contains(&metadata.revision) {
      return Err(CodecError::Message(format!(
        "UDF metadata declares unsupported revision {:#06x}",
        metadata.revision
      )));
    }
    validate_metadata(&metadata)?;

    let supplied: BTreeMap<_, _> = files
      .iter()
      .map(|file| (file.name.as_str(), file))
      .collect();
    if supplied.len() != files.len() {
      return Err(CodecError::Message(
        "The artifacts contain duplicate names".to_owned(),
      ));
    }
    let expected: BTreeSet<_> = metadata
      .files
      .iter()
      .map(|file| file.artifact.as_str())
      .collect();
    let actual: BTreeSet<_> = supplied.keys().copied().collect();
    if actual != expected {
      return Err(CodecError::Message(format!(
        "The UDF metadata describes {} file artifacts, but encode received {}",
        expected.len(),
        actual.len()
      )));
    }

    for file in &metadata.files {
      let artifact = supplied.get(file.artifact.as_str()).ok_or_else(|| {
        CodecError::Message(format!(
          "UDF metadata references missing artifact '{}'",
          file.artifact
        ))
      })?;
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
      if artifact.data.len() > capacity {
        return Err(CodecError::Message(format!(
          "UDF file '{}' needs {} bytes, but its existing allocation holds {capacity}; growing beyond allocated extents is unsupported",
          file.artifact,
          artifact.data.len()
        )));
      }
    }

    if let Some(bridge) = metadata.bridge {
      for iso_file in &bridge.iso_files {
        let artifact = supplied.get(iso_file.artifact.as_str()).ok_or_else(|| {
          CodecError::Message(format!(
            "ISO 9660 metadata references missing artifact '{}'",
            iso_file.artifact
          ))
        })?;
        let iso_capacity = iso_file.length.div_ceil(BLOCK_SIZE) * BLOCK_SIZE;
        if artifact.data.len() > iso_capacity {
          return Err(CodecError::Message(format!(
            "File '{}' needs {} bytes, exceeding the ISO 9660 extent's existing {iso_capacity} bytes; moving or allocating blocks is unsupported for a bridge",
            iso_file.artifact,
            artifact.data.len()
          )));
        }
      }

      let mut iso_artifacts = vec![DecodedArtifact {
        name: "ISO9660.json".to_owned(),
        data: bridge.iso_metadata,
      }];

      for iso_file in &bridge.iso_files {
        let file = supplied.get(iso_file.artifact.as_str()).ok_or_else(|| {
          CodecError::Message(format!(
            "ISO 9660 metadata references missing artifact '{}'",
            iso_file.artifact
          ))
        })?;
        if metadata
          .files
          .iter()
          .find(|udf_file| udf_file.artifact == iso_file.artifact)
          .is_some_and(|udf_file| udf_file.iso_name.as_deref() != Some(iso_file.name.as_str()))
        {
          return Err(CodecError::Message(format!(
            "UDF artifact '{}' has an inconsistent ISO 9660 mapping",
            iso_file.artifact
          )));
        }
        iso_artifacts.push(DecodedArtifact {
          name: iso_file.name.clone(),
          data: file.data.clone(),
        });
      }

      let iso_args = match placement {
        Placement::Shift => Some(r#"{"placement":"shift"}"#),
        Placement::Relocate => Some(r#"{"placement":"relocate"}"#),
        Placement::Preserve => Some(r#"{"placement":"preserve"}"#),
      };
      let mut rebuilt = Iso9660.encode(Vec::new(), &iso_artifacts, iso_args)?;
      let rebuilt_iso = iso_volume::read(
        &rebuilt,
        crate::codecs::cdrom::iso9660::OutOfBoundsEntries::Preserve,
      )?;
      for iso_file in &bridge.iso_files {
        let expected_extent = iso_file.extent / BLOCK_SIZE as u64;
        let current = rebuilt_iso
          .files
          .iter()
          .find(|file| file.name == iso_file.name)
          .ok_or_else(|| {
            CodecError::Message(format!(
              "Rebuilt ISO 9660 image lost file '{}'",
              iso_file.name
            ))
          })?;
        if u64::from(current.extent) != expected_extent {
          return Err(CodecError::Message(format!(
            "ISO 9660 moved '{}' from block {expected_extent} to block {}; the UDF reference cannot be updated safely",
            iso_file.name, current.extent
          )));
        }
      }

      for file in &metadata.files {
        let artifact = supplied[file.artifact.as_str()];
        if file.iso_name.is_none() {
          write_file_extents(&mut rebuilt, file, &artifact.data)?;
        }
        if artifact.data.len() != file.length {
          update_file_entries(&mut rebuilt, file, artifact.data.len())?;
        }
      }

      let rebuilt_udf = read_udf(&rebuilt)?;
      let rebuilt_iso = iso_volume::read(
        &rebuilt,
        crate::codecs::cdrom::iso9660::OutOfBoundsEntries::Preserve,
      )?;
      validate_bridge_allocations(&rebuilt, &rebuilt_udf, &rebuilt_iso)?;
      validate_rebuilt_udf_files(&metadata.files, &supplied, &rebuilt_udf)?;
      for iso_file in &bridge.iso_files {
        let expected = supplied.get(iso_file.artifact.as_str()).ok_or_else(|| {
          CodecError::Message(format!(
            "ISO 9660 metadata references missing artifact '{}'",
            iso_file.artifact
          ))
        })?;
        let actual = rebuilt_iso
          .files
          .iter()
          .find(|file| file.name == iso_file.name)
          .ok_or_else(|| {
            CodecError::Message(format!(
              "Rebuilt ISO 9660 image lost file '{}'",
              iso_file.name
            ))
          })?;
        if actual.data != expected.data {
          return Err(CodecError::Message(format!(
            "Rebuilt ISO 9660 file '{}' does not match artifact '{}'",
            iso_file.name, iso_file.artifact
          )));
        }
      }

      return Ok(rebuilt);
    }

    let mut image = vec![0; metadata.image_len];
    for run in &metadata.opaque {
      let start = usize::try_from(run.offset).map_err(|_| {
        CodecError::Message("UDF opaque offset does not fit this machine".to_owned())
      })?;
      let end = start
        .checked_add(run.data.len())
        .ok_or_else(|| CodecError::Message("UDF opaque range overflows the image".to_owned()))?;
      let target = image
        .get_mut(start..end)
        .ok_or_else(|| CodecError::Message("UDF opaque range lies outside the image".to_owned()))?;
      target.copy_from_slice(run.data.as_bytes());
    }

    for file in &metadata.files {
      let artifact = supplied.get(file.artifact.as_str()).ok_or_else(|| {
        CodecError::Message(format!(
          "UDF metadata references missing artifact '{}'",
          file.artifact
        ))
      })?;
      write_file_extents(&mut image, file, &artifact.data)?;
      if artifact.data.len() != file.length {
        update_file_entries(&mut image, file, artifact.data.len())?;
      }
    }

    let rebuilt_udf = read_udf(&image)?;
    validate_rebuilt_udf_files(&metadata.files, &supplied, &rebuilt_udf)?;

    Ok(image)
  }
}

fn image_bytes_at<'a>(
  image: &'a [u8],
  offset: u64,
  length: usize,
  name: &str,
) -> Result<&'a [u8], CodecError> {
  let start = usize::try_from(offset)
    .map_err(|_| CodecError::Message(format!("File '{name}' offset does not fit this machine")))?;
  let end = start
    .checked_add(length)
    .ok_or_else(|| CodecError::Message(format!("File '{name}' range overflows")))?;
  image
    .get(start..end)
    .ok_or_else(|| CodecError::Message(format!("File '{name}' lies outside the input image")))
}
