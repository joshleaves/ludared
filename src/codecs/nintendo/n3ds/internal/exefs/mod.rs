//! Nintendo 3DS ExeFS (Executable Filesystem) codec.
//!
//! Extracts the files of an ExeFS image and rebuilds it. File hashes are
//! recomputed from the artifact bytes on rebuild; the original layout (file
//! offsets) is preserved when it still fits the (possibly edited) artifacts,
//! and recomputed with the makerom layout (sequential, 0x200-aligned)
//! otherwise.
//!
//! Format reference: <https://www.3dbrew.org/wiki/ExeFS>.

use crate::codecs::common::bytes::{read_field as field, read_u32_le as u32_le};
use crate::codecs::common::hash::sha256;
use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::n3ds::common::hex_field;
use crate::codecs::{Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact, LudaredCodecArgs};
use log::*;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

const CODEC_ID: &str = "std/nintendo/3ds/internal/exefs";
const CODEC_NAME: &str = "Nintendo 3DS ExeFS";
const CODEC_DESC: &str = r#"Nintendo 3DS ExeFS (Executable Filesystem)

Extracts an ExeFS image into:
- ExeFS.json: header metadata, including the original file layout and any
  non-zero padding, so an unmodified rebuild stays byte-identical.
- One artifact per file, under its ExeFS name (.code, icon, banner, ...).

Rebuilding recomputes the SHA-256 file hashes from the artifact bytes, so
edited files stay consistent with the header. The original file offsets are
reused while they still fit the artifacts; once a file changes size the layout
is recomputed, placing files sequentially on 0x200-byte boundaries.

This codec works on cleartext ExeFS images. An ExeFS extracted from an
encrypted NCCH must be decrypted first; the codec does not perform the AES-CTR
encryption of the parent container.

No arguments.
"#;

const HEADER_SIZE: usize = 0x200;
const MAX_FILES: usize = 10;
const FILE_HEADER_SIZE: usize = 0x10;
const NAME_SIZE: usize = 8;
const RESERVED_OFFSET: usize = 0xA0;
const RESERVED_SIZE: usize = 0x20;
const HASHES_OFFSET: usize = 0xC0;
const HASH_SIZE: usize = 0x20;
const FILE_ALIGN: u64 = 0x200;
const METADATA_NAME: &str = "ExeFS.json";

pub(crate) struct N3dsExeFs;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct N3dsExeFsCodecArgs {}

impl LudaredCodecArgs for N3dsExeFsCodecArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileMetadata {
  /// UTF-8 view of the 8-byte ExeFS name.
  name: String,
  /// Raw 8-byte name, recorded when it is not the canonical NUL-padded form.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  name_raw: Option<String>,
  /// Offset from the end of the 0x200-byte header, as decoded.
  offset: u64,
  /// Non-zero gap bytes found between the previous file and this one.
  /// Zero gaps are implicit from the offsets and not recorded.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  gap: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExefsMetadata {
  /// The 0x20 reserved bytes at 0xA0, hex-encoded.
  reserved: String,
  /// Zero padding after the last file.
  trailing_zeros: u64,
  /// Non-zero bytes after the last file, when present.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  trailing: Option<String>,
  files: Vec<FileMetadata>,
}

impl ExefsMetadata {
  fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the ExeFS metadata: {err}")))
  }

  fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the ExeFS metadata artifact: {err}"))
    })
  }
}

#[derive(Debug, Clone)]
struct FileHeader {
  name_raw: [u8; NAME_SIZE],
  name: String,
  offset: u64,
  size: u64,
}

/// Reads the fixed-size header shared by `can_handle` and `decode`.
///
/// Returns `None` when the header cannot be an ExeFS header at all; structural
/// oddities that still parse are reported through `issues`.
fn parse_file_headers(data: &[u8]) -> Option<(Vec<FileHeader>, [u8; RESERVED_SIZE])> {
  let header = data.get(..HEADER_SIZE)?;
  let reserved: [u8; RESERVED_SIZE] = field(header, RESERVED_OFFSET).ok()?;

  let mut files = Vec::new();
  let mut unused_seen = false;
  for index in 0..MAX_FILES {
    let base = index * FILE_HEADER_SIZE;
    let name_raw: [u8; NAME_SIZE] = field(header, base).ok()?;
    let offset = u32_le(header, base + 0x8).ok()?;
    let size = u32_le(header, base + 0xC).ok()?; // A zero-length file at offset zero may still have a non-empty name.
    let used = name_raw != [0; NAME_SIZE] || offset != 0 || size != 0;
    if !used {
      unused_seen = true;
      continue;
    }
    if unused_seen {
      // Used entries must be contiguous: a hole means this is not an ExeFS.
      return None;
    }

    // A used name is NUL-padded; the first NUL ends the name.
    let end = name_raw
      .iter()
      .position(|&byte| byte == 0)
      .unwrap_or(NAME_SIZE);
    let name = std::str::from_utf8(&name_raw[..end]).ok()?.to_owned();
    if name.is_empty() {
      return None;
    }

    files.push(FileHeader {
      name_raw,
      name,
      offset: u64::from(offset),
      size: u64::from(size),
    });
  }

  Some((files, reserved))
}

/// Checks that the files fit in `data_len` without overlapping, in header
/// order, and returns the end of the last file.
fn layout_end(files: &[FileHeader], data_len: u64) -> Option<u64> {
  let mut end = 0u64;
  for file in files {
    if file.offset < end {
      return None;
    }
    end = file.offset.checked_add(file.size)?;
  }
  if end > data_len {
    return None;
  }
  Some(end)
}

fn hash_slot(index: usize) -> usize {
  // Hashes are stored in reverse order: the first file's hash ends the header.
  HASHES_OFFSET + (MAX_FILES - 1 - index) * HASH_SIZE
}

fn name_artifact_error(name: &str) -> Option<CodecError> {
  if name.is_empty()
    || name.len() > NAME_SIZE
    || name == METADATA_NAME
    || name == "."
    || name == ".."
    || name.contains('/')
    || name.contains('\\')
  {
    return Some(CodecError::Message(format!(
      "ExeFS file name {name:?} cannot be used as an artifact name (names must fit in {NAME_SIZE} bytes and cannot be paths or reserved names)"
    )));
  }
  None
}

fn has_duplicate_names(files: &[FileHeader]) -> bool {
  let mut names = std::collections::HashSet::with_capacity(files.len());
  files.iter().any(|file| !names.insert(file.name.as_str()))
}

impl Codec for N3dsExeFs {
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
    let Some((files, reserved)) = parse_file_headers(data) else {
      return Ok(CodecHandlingConfidence::No);
    };
    if files.is_empty()
      || has_duplicate_names(&files)
      || layout_end(&files, (data.len() - HEADER_SIZE) as u64).is_none()
    {
      return Ok(CodecHandlingConfidence::No);
    }
    if files
      .iter()
      .any(|file| name_artifact_error(&file.name).is_some())
    {
      return Ok(CodecHandlingConfidence::No);
    }

    // The strongest available proof: every file matches its recorded hash.
    // Retail ExeFS files are encrypted under different keys, so a mismatch is
    // common on raw container cuts and only lowers confidence.
    let hashes_match = files.iter().enumerate().all(|(index, file)| {
      let Some(body_start) = usize::try_from(file.offset).ok() else {
        return false;
      };
      let Some(body_end) = body_start.checked_add(file.size as usize) else {
        return false;
      };
      let Some(start) = HEADER_SIZE.checked_add(body_start) else {
        return false;
      };
      let Some(end) = HEADER_SIZE.checked_add(body_end) else {
        return false;
      };
      data.get(start..end).is_some_and(|bytes| {
        sha256(bytes) == field::<HASH_SIZE>(data, hash_slot(index)).unwrap_or([0; HASH_SIZE])
      })
    });
    if hashes_match {
      return Ok(CodecHandlingConfidence::Certain);
    }

    let canonical = reserved == [0; RESERVED_SIZE]
      && files.iter().all(|file| file.offset % FILE_ALIGN == 0)
      && files
        .iter()
        .all(|file| file.name_raw == canonical_name(&file.name));
    Ok(if canonical {
      CodecHandlingConfidence::Likely
    } else {
      CodecHandlingConfidence::Possible
    })
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("exefs".to_owned())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding Nintendo 3DS ExeFS image");
    let _args = N3dsExeFsCodecArgs::from_args(args)?;

    let Some((files, reserved)) = parse_file_headers(data) else {
      return Err(CodecError::Message(
        "Not an ExeFS image: the file headers are malformed".to_owned(),
      ));
    };
    if files.is_empty() {
      return Err(CodecError::Message(
        "Not an ExeFS image: the header declares no files".to_owned(),
      ));
    }
    if has_duplicate_names(&files) {
      return Err(CodecError::Message(
        "ExeFS header contains duplicate file names, which cannot produce unique artifacts"
          .to_owned(),
      ));
    }
    let body_len = data.len() - HEADER_SIZE;
    let Some(files_end) = layout_end(&files, body_len as u64) else {
      return Err(CodecError::Message(
        "ExeFS files overlap or exceed the image size".to_owned(),
      ));
    };

    let mut metadata = ExefsMetadata {
      reserved: hex::encode(reserved),
      trailing_zeros: 0,
      trailing: None,
      files: Vec::with_capacity(files.len()),
    };

    let mut artifacts = Vec::with_capacity(files.len() + 1);
    let body = &data[HEADER_SIZE..];
    let mut previous_end = 0u64;
    for file in &files {
      if let Some(error) = name_artifact_error(&file.name) {
        return Err(error);
      }
      let gap = body
        .get(previous_end as usize..file.offset as usize)
        .ok_or_else(|| {
          CodecError::Message(format!(
            "ExeFS gap before {} is outside the image",
            file.name
          ))
        })?;
      let gap = (!gap.iter().all(|&byte| byte == 0)).then(|| hex::encode(gap));

      metadata.files.push(FileMetadata {
        name: file.name.clone(),
        name_raw: (file.name_raw != canonical_name(&file.name)).then(|| hex::encode(file.name_raw)),
        offset: file.offset,
        gap,
      });

      let file_end = file
        .offset
        .checked_add(file.size)
        .ok_or_else(|| CodecError::Message(format!("ExeFS file {} range overflows", file.name)))?;
      let bytes = body
        .get(file.offset as usize..file_end as usize)
        .ok_or_else(|| {
          CodecError::Message(format!("ExeFS file {} is outside the image", file.name))
        })?
        .to_vec();
      artifacts.push(DecodedArtifact {
        name: file.name.clone(),
        data: bytes,
      });

      previous_end = file.offset + file.size;
    }

    let trailing = body
      .get(files_end as usize..)
      .ok_or_else(|| CodecError::Message("ExeFS trailing data is outside the image".to_owned()))?;
    if trailing.iter().all(|&byte| byte == 0) {
      metadata.trailing_zeros = trailing.len() as u64;
    } else if !trailing.is_empty() {
      metadata.trailing = Some(hex::encode(trailing));
    }

    artifacts.insert(
      0,
      DecodedArtifact {
        name: METADATA_NAME.to_owned(),
        data: metadata.to_json()?,
      },
    );
    Ok(artifacts)
  }

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding Nintendo 3DS ExeFS image");
    let _args = N3dsExeFsCodecArgs::from_args(args)?;

    let (metadata_artifact, files) = artifacts.split_first().ok_or_else(|| {
      CodecError::Message(format!("Expected a {METADATA_NAME} artifact, found none"))
    })?;
    if metadata_artifact.name != METADATA_NAME {
      return Err(CodecError::Message(format!(
        "Expected the first artifact to be {METADATA_NAME}, found {}",
        metadata_artifact.name
      )));
    }
    let metadata = ExefsMetadata::from_json(&metadata_artifact.data)?;

    if metadata.files.is_empty() || metadata.files.len() > MAX_FILES {
      return Err(CodecError::Message(format!(
        "ExeFS metadata describes {} files (expected between 1 and {MAX_FILES})",
        metadata.files.len()
      )));
    }

    let reserved = hex_field(&metadata.reserved, "reserved", RESERVED_SIZE)?;
    let trailing = metadata
      .trailing
      .as_deref()
      .map(|text| {
        hex::decode(text).map_err(|err| {
          CodecError::Message(format!(
            "ExeFS metadata trailing is not valid hexadecimal: {err}"
          ))
        })
      })
      .transpose()?;

    // Names must be representable in the fixed-width header and unique as
    // artifact paths before matching bytes to metadata.
    let mut metadata_names = std::collections::HashSet::with_capacity(metadata.files.len());
    for file in &metadata.files {
      if let Some(error) = name_artifact_error(&file.name) {
        return Err(error);
      }
      if !metadata_names.insert(file.name.as_str()) {
        return Err(CodecError::DuplicateArtifact(file.name.clone()));
      }
    }

    // Match the artifacts to the metadata: every declared file exactly once.
    let mut contents: Vec<&[u8]> = Vec::with_capacity(metadata.files.len());
    for file in &metadata.files {
      let matching: Vec<&[u8]> = files
        .iter()
        .filter(|artifact| artifact.name == file.name)
        .map(|artifact| artifact.data.as_slice())
        .collect();
      match matching.as_slice() {
        [] => {
          return Err(CodecError::Message(format!(
            "Expected a {} artifact, found none",
            file.name
          )));
        }
        [bytes] => contents.push(bytes),
        _ => return Err(CodecError::DuplicateArtifact(file.name.clone())),
      }
    }
    for artifact in files {
      if !metadata.files.iter().any(|file| file.name == artifact.name) {
        return Err(CodecError::Message(format!(
          "Unexpected artifact: {}",
          artifact.name
        )));
      }
    }

    // Reuse the decoded layout while it still fits the artifacts; once a file
    // changed size, recompute it with the makerom layout.
    let mut offsets: Vec<u64> = Vec::with_capacity(metadata.files.len());
    let mut end = 0u64;
    let mut reusable = true;
    for (file, bytes) in metadata.files.iter().zip(&contents) {
      if file.offset < end {
        reusable = false;
        break;
      }
      offsets.push(file.offset);
      let Some(next_end) = file.offset.checked_add(bytes.len() as u64) else {
        reusable = false;
        break;
      };
      end = next_end;
    }
    if !reusable {
      offsets.clear();
      end = 0;
      for bytes in &contents {
        let offset = end
          .checked_add(FILE_ALIGN - 1)
          .map(|value| value / FILE_ALIGN * FILE_ALIGN)
          .filter(|_| end != 0)
          .unwrap_or(0);
        offsets.push(offset);
        end = offset + bytes.len() as u64;
      }
      debug!("ExeFS layout recomputed: the original offsets no longer fit");
    }

    let mut header = [0u8; HEADER_SIZE];
    header[RESERVED_OFFSET..RESERVED_OFFSET + RESERVED_SIZE].copy_from_slice(&reserved);

    let mut image = Vec::new();
    let mut position = 0u64;
    for (index, ((file, bytes), offset)) in metadata
      .files
      .iter()
      .zip(&contents)
      .zip(&offsets)
      .enumerate()
    {
      let name_raw = match &file.name_raw {
        Some(raw) => {
          let raw = hex_field(raw, "file name", NAME_SIZE)?;
          let end = raw.iter().position(|&byte| byte == 0).unwrap_or(NAME_SIZE);
          if raw[..end] != *file.name.as_bytes() {
            return Err(CodecError::Message(format!(
              "ExeFS metadata raw name of {} does not match its text name",
              file.name
            )));
          }
          raw
        }
        None => {
          let raw = canonical_name(&file.name);
          if raw[..file.name.len()] != *file.name.as_bytes() {
            return Err(CodecError::Message(format!(
              "ExeFS file name {:?} is not 8-bit encodable",
              file.name
            )));
          }
          raw.to_vec()
        }
      };

      let gap = offset - position;
      match &file.gap {
        Some(gap_hex) => {
          let gap_bytes = hex::decode(gap_hex).map_err(|err| {
            CodecError::Message(format!(
              "ExeFS metadata gap of {} is not valid hexadecimal: {err}",
              file.name
            ))
          })?;
          if gap_bytes.len() as u64 != gap {
            return Err(CodecError::Message(format!(
              "ExeFS metadata gap of {} is {} bytes, expected {gap}",
              file.name,
              gap_bytes.len()
            )));
          }
          image.extend_from_slice(&gap_bytes);
        }
        None => image.extend(std::iter::repeat_n(0, gap as usize)),
      }

      let base = index * FILE_HEADER_SIZE;
      header[base..base + NAME_SIZE].copy_from_slice(&name_raw);
      header[base + 0x8..base + 0xC].copy_from_slice(
        &u32::try_from(*offset)
          .map_err(|_| {
            CodecError::Message(format!(
              "ExeFS file {} offset does not fit the header",
              file.name
            ))
          })?
          .to_le_bytes(),
      );
      header[base + 0xC..base + 0x10].copy_from_slice(
        &u32::try_from(bytes.len())
          .map_err(|_| {
            CodecError::Message(format!(
              "ExeFS file {} size does not fit the header",
              file.name
            ))
          })?
          .to_le_bytes(),
      );

      header[hash_slot(index)..hash_slot(index) + HASH_SIZE].copy_from_slice(&sha256(bytes));

      image.extend_from_slice(bytes);
      position = offset + bytes.len() as u64;
    }

    if let Some(trailing) = trailing {
      image.extend_from_slice(&trailing);
    }
    image.extend(std::iter::repeat_n(0, metadata.trailing_zeros as usize));

    let mut result = Vec::with_capacity(HEADER_SIZE + image.len());
    result.extend_from_slice(&header);
    result.extend_from_slice(&image);
    Ok(result)
  }
}

/// The canonical 8-byte form of an ExeFS name: the name bytes, NUL-padded.
fn canonical_name(name: &str) -> [u8; NAME_SIZE] {
  let mut raw = [0u8; NAME_SIZE];
  let bytes = name.as_bytes();
  let len = bytes.len().min(NAME_SIZE);
  raw[..len].copy_from_slice(&bytes[..len]);
  raw
}
