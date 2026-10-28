//! Nintendo 3DS BCFNT font codec.
//!
//! A BCFNT file is a small archive: a header followed by FINF (font info),
//! TGLP (glyph texture), CWDH (character widths) and CMAP (character mapping)
//! sections. Decode exposes the readable sections as JSON and each texture
//! sheet as one artifact: an editable PNG where the format allows it, or the
//! native bytes otherwise.

pub(crate) mod metadata;
mod texture;

#[cfg(test)]
mod tests;

use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::n3ds::common::field;
use crate::codecs::{Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact, LudaredCodecArgs};
use log::trace;
use metadata::{
  BcfntMetadata, ByteOrder, CmapEntry, CmapMetadata, CmapSection, CwdhEntry, CwdhMetadata,
  CwdhSection, FontInfo, FontMagic, TextureFormat, TextureInfo, decode_hex, encode_hex,
  texture_artifact, texture_name,
};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use texture::{decode_sheet, encode_sheet, png_to_sheet, sheet_size, sheet_to_png};

const CODEC_ID: &str = "std/nintendo/font/bcfnt";
const CODEC_NAME: &str = "Nintendo 3DS BCFNT font";
const CODEC_DESC: &str = r#"Supports RGBA8, RGB8, RGBA5551, RGB565, RGBA4, LA8, HILO8 (raw only), L8, A8, LA4, L4, A4, ETC1 and ETC1A4 textures.

Warning: Bit-exact reconstruction is not guaranteed for ETC1/ETC1A4 textures due to lossy recompression."#;

const HEADER_SIZE: usize = 0x14;
const FINF_SIZE: usize = 0x20;
const TGLP_SIZE: usize = 0x20;
const CWDH_SIZE: usize = 0x10;
const CMAP_SIZE: usize = 0x14;
const VERSION: u32 = 0x0300_0000;
const MAX_FILE_SIZE: usize = 256 * 1024 * 1024;
// Leave room for simultaneous pixel and PNG buffers during texture conversion.
const MAX_EXPANDED_IMAGE_SIZE: usize = 64 * 1024 * 1024;

const FINF_OFFSET: usize = HEADER_SIZE;
const TGLP_OFFSET: usize = FINF_OFFSET + FINF_SIZE;

pub(crate) struct N3dsBcfnt;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct N3dsBcfntCodecArgs {}

impl LudaredCodecArgs for N3dsBcfntCodecArgs {}

impl Codec for N3dsBcfnt {
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
    if data.len() > MAX_FILE_SIZE {
      return Ok(CodecHandlingConfidence::No);
    }

    let Ok(header) = Header::parse(data) else {
      return Ok(CodecHandlingConfidence::No);
    };
    let Ok(finf) = Finf::parse(data, &header) else {
      return Ok(CodecHandlingConfidence::Possible);
    };
    let Ok((start, section_size)) = tglp_extent(data, &header, &finf) else {
      return Ok(CodecHandlingConfidence::Possible);
    };

    Ok(if validate_complete(data, &header, &finf).is_ok() {
      CodecHandlingConfidence::Certain
    } else if start >= TGLP_OFFSET && section_size >= TGLP_SIZE {
      CodecHandlingConfidence::Likely
    } else {
      CodecHandlingConfidence::Possible
    })
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("bcfnt".to_owned())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding Nintendo 3DS BCFNT font");
    let _args = N3dsBcfntCodecArgs::from_args(args)?;

    validate_file_size(data.len())?;

    let header = Header::parse(data)?;
    let finf = Finf::parse(data, &header)?;
    let tglp = Tglp::parse(data, &header, &finf, true)?;
    let (cwdh, cwdh_end) = parse_cwdh_chain(data, header.order, finf.cwdh_offset, tglp.end)?;
    let (cmap, cmap_end) = parse_cmap_chain(data, header.order, finf.cmap_offset, tglp.end)?;

    let section_count = 2usize
      .checked_add(cwdh.sections.len())
      .and_then(|count| count.checked_add(cmap.sections.len()))
      .ok_or_else(|| CodecError::Message("BCFNT section count overflows".to_owned()))?;
    if usize::try_from(header.sections).ok() != Some(section_count) {
      return Err(CodecError::Message(format!(
        "BCFNT header declares {} sections, but the file holds {section_count}",
        header.sections
      )));
    }
    let file_end = cwdh_end.max(cmap_end).max(tglp.end);

    let texture = tglp.texture_info()?;

    let metadata = BcfntMetadata {
      magic: header.magic,
      byte_order: header.order,
      version: header.version,
      font: finf.font,
      texture,
      tglp_gap: Some(encode_hex(&tglp.gap)),
      tglp_trailing: Some(encode_hex(&tglp.trailing)),
      file_trailer: Some(encode_hex(data.get(file_end..).ok_or_else(|| {
        CodecError::Message("BCFNT file trailer offset is invalid".to_owned())
      })?)),
    };

    let mut artifacts = Vec::with_capacity(
      3usize
        .checked_add(tglp.sheets.len())
        .ok_or_else(|| CodecError::Message("BCFNT artifact count overflows".to_owned()))?,
    );

    artifacts.push(DecodedArtifact {
      name: metadata::METADATA_NAME.to_owned(),
      data: metadata.to_json()?,
    });
    artifacts.push(DecodedArtifact {
      name: metadata::CWDH_NAME.to_owned(),
      data: cwdh.to_json()?,
    });
    artifacts.push(DecodedArtifact {
      name: metadata::CMAP_NAME.to_owned(),
      data: cmap.to_json()?,
    });

    let png = metadata
      .texture
      .format
      .has_png_artifact(metadata.texture.width, metadata.texture.height);
    for (index, sheet) in tglp.sheets.into_iter().enumerate() {
      artifacts.push(DecodedArtifact {
        name: texture_name(index, png),
        data: sheet,
      });
    }

    Ok(artifacts)
  }

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding Nintendo 3DS BCFNT font");
    let _args = N3dsBcfntCodecArgs::from_args(args)?;

    let sorted = sort_artifacts(artifacts)?;
    let metadata = BcfntMetadata::from_json(sorted.metadata)?;
    let cwdh = CwdhMetadata::from_json(sorted.cwdh)?;
    let cmap = CmapMetadata::from_json(sorted.cmap)?;

    let texture = &metadata.texture;
    let sheet_count = sorted.textures.len();
    let sheet_size = if sheet_count == 0 {
      0
    } else {
      ensure_expanded_image_size(texture.width, texture.height, sheet_count)?;
      sheet_size(texture.format, texture.width, texture.height)?
    };
    let all_sheet_bytes = sheet_size
      .checked_mul(sheet_count)
      .filter(|bytes| *bytes <= MAX_FILE_SIZE)
      .ok_or_else(|| {
        CodecError::Message("BCFNT texture sheets exceed the 256 MiB safety limit".to_owned())
      })?;
    let sheets = build_sheets(texture, metadata.byte_order, &sorted.textures)?;
    let tglp_gap = decode_hex("TGLP gap", metadata.tglp_gap.as_deref().unwrap_or(""))?;
    let tglp_trailing = decode_hex(
      "TGLP trailing bytes",
      metadata.tglp_trailing.as_deref().unwrap_or(""),
    )?;
    let file_trailer = decode_hex(
      "file trailer",
      metadata.file_trailer.as_deref().unwrap_or(""),
    )?;

    let header_end = tglp_data_offset()?;
    let data_offset = header_end
      .checked_add(tglp_gap.len())
      .ok_or_else(|| CodecError::Message("BCFNT TGLP data offset overflows".to_owned()))?;
    let sheet_bytes = all_sheet_bytes;
    let sheets_end = data_offset
      .checked_add(sheet_bytes)
      .ok_or_else(|| CodecError::Message("BCFNT texture sheets overflow".to_owned()))?;
    let tglp_end = sheets_end
      .checked_add(tglp_trailing.len())
      .ok_or_else(|| CodecError::Message("BCFNT TGLP trailing data overflows".to_owned()))?;
    let tglp_section_size = tglp_end
      .checked_sub(TGLP_OFFSET)
      .ok_or_else(|| CodecError::Message("BCFNT TGLP section underflows".to_owned()))?;

    let cwdh_bytes = encode_cwdh_sections(&cwdh.sections, tglp_end, metadata.byte_order)?;
    let cwdh_end = cwdh_bytes.last().map_or(tglp_end, |(_, end, _)| *end);
    let cmap_bytes = encode_cmap_sections(&cmap.sections, cwdh_end, metadata.byte_order)?;
    let sections_end = cmap_bytes.last().map_or(cwdh_end, |(_, end, _)| *end);
    let file_size = sections_end
      .checked_add(file_trailer.len())
      .ok_or_else(|| CodecError::Message("BCFNT file trailer overflows".to_owned()))?;

    if file_size > MAX_FILE_SIZE {
      return Err(CodecError::Message(format!(
        "Rebuilt BCFNT is {file_size} bytes; maximum supported size is {MAX_FILE_SIZE} bytes"
      )));
    }
    let file_size_u32 = u32::try_from(file_size)
      .map_err(|_| CodecError::Message("BCFNT file is too large to encode".to_owned()))?;
    let section_count_u32 = u32::try_from(
      2usize
        .checked_add(cwdh.sections.len())
        .and_then(|count| count.checked_add(cmap.sections.len()))
        .ok_or_else(|| CodecError::Message("BCFNT section count overflows".to_owned()))?,
    )
    .map_err(|_| CodecError::Message("BCFNT section count is too large".to_owned()))?;

    let tglp_data_offset = u32::try_from(TGLP_OFFSET + 8)
      .map_err(|_| CodecError::Message("BCFNT TGLP offset is too large".to_owned()))?;
    let cwdh_data_offset = cwdh_bytes
      .first()
      .map(|(start, _, _)| {
        u32::try_from(start + 8)
          .map_err(|_| CodecError::Message("BCFNT CWDH offset is too large".to_owned()))
      })
      .transpose()?
      .unwrap_or(0);
    let cmap_data_offset = cmap_bytes
      .first()
      .map(|(start, _, _)| {
        u32::try_from(start + 8)
          .map_err(|_| CodecError::Message("BCFNT CMAP offset is too large".to_owned()))
      })
      .transpose()?
      .unwrap_or(0);

    let mut out = Vec::with_capacity(file_size);
    write_header(&mut out, &metadata, file_size_u32, section_count_u32)?;
    write_finf(
      &mut out,
      &metadata.font,
      tglp_data_offset,
      cwdh_data_offset,
      cmap_data_offset,
      metadata.byte_order,
    )?;
    write_tglp_header(
      &mut out,
      texture,
      sheet_size,
      sheet_count,
      tglp_section_size,
      data_offset,
      metadata.byte_order,
    )?;

    if out.len() > data_offset {
      return Err(CodecError::Message(
        "BCFNT TGLP header overlaps the texture sheet data".to_owned(),
      ));
    }

    out.resize(header_end, 0);
    out.extend_from_slice(&tglp_gap);
    for sheet in sheets {
      out.extend_from_slice(&sheet);
    }
    out.extend_from_slice(&tglp_trailing);

    for (_, _, bytes) in cwdh_bytes {
      out.extend_from_slice(&bytes.bytes);
    }
    for (_, _, bytes) in cmap_bytes {
      out.extend_from_slice(&bytes.bytes);
    }

    out.extend_from_slice(&file_trailer);

    if out.len() > MAX_FILE_SIZE {
      return Err(CodecError::Message(format!(
        "Rebuilt BCFNT is {} bytes; maximum supported size is {MAX_FILE_SIZE} bytes",
        out.len()
      )));
    }

    if out.len() != file_size {
      return Err(CodecError::Message(format!(
        "BCFNT encoder produced {} bytes, expected {file_size}",
        out.len()
      )));
    }

    Ok(out)
  }
}

struct Header {
  magic: FontMagic,
  order: ByteOrder,
  version: u32,
  sections: u32,
}

impl Header {
  fn parse(data: &[u8]) -> Result<Self, CodecError> {
    let header = data.get(..HEADER_SIZE).ok_or_else(|| {
      CodecError::Message(format!(
        "BCFNT header is truncated: {} bytes (minimum: {HEADER_SIZE})",
        data.len()
      ))
    })?;

    let magic_bytes = field::<4>(header, 0)?;
    let magic = FontMagic::from_bytes(magic_bytes)
      .ok_or_else(|| CodecError::Message("Not a BCFNT font: the magic is missing".to_owned()))?;

    let bom = field::<2>(header, 4)?;
    let order = match bom {
      [0xFF, 0xFE] => ByteOrder::Little,
      [0xFE, 0xFF] => ByteOrder::Big,
      _ => {
        return Err(CodecError::Message(format!(
          "Invalid BCFNT byte-order marker: {bom:02x?}"
        )));
      }
    };

    if read_u16(header, 6, order)? != HEADER_SIZE as u16 {
      return Err(CodecError::Message(format!(
        "Invalid BCFNT header size: {} (expected {HEADER_SIZE})",
        read_u16(header, 6, order)?
      )));
    }

    let version = read_u32(header, 8, order)?;
    if version != VERSION {
      return Err(CodecError::Message(format!(
        "Unsupported BCFNT version {version:#010x} (expected {VERSION:#010x})"
      )));
    }

    let declared_size = read_u32(header, 0x0C, order)?;
    let actual_size = u32::try_from(data.len())
      .map_err(|_| CodecError::Message("BCFNT file is too large".to_owned()))?;
    if declared_size != actual_size {
      return Err(CodecError::Message(format!(
        "BCFNT header declares {declared_size} bytes, but the file is {actual_size} bytes"
      )));
    }

    let sections = read_u32(header, 0x10, order)?;

    Ok(Self {
      magic,
      order,
      version,
      sections,
    })
  }
}

struct Finf {
  font: FontInfo,
  tglp_offset: u32,
  cwdh_offset: u32,
  cmap_offset: u32,
}

impl Finf {
  fn parse(data: &[u8], header: &Header) -> Result<Self, CodecError> {
    let section = data
      .get(FINF_OFFSET..FINF_OFFSET + FINF_SIZE)
      .ok_or_else(|| CodecError::Message("BCFNT FINF section is truncated".to_owned()))?;

    if field::<4>(section, 0)? != *b"FINF" {
      return Err(CodecError::Message(
        "Not a BCFNT font: the FINF magic is missing".to_owned(),
      ));
    }

    if read_u32(section, 4, header.order)? != FINF_SIZE as u32 {
      return Err(CodecError::Message(format!(
        "Invalid BCFNT FINF size: {} (expected {FINF_SIZE})",
        read_u32(section, 4, header.order)?
      )));
    }

    Ok(Self {
      font: FontInfo {
        font_type: read_u8(section, 0x08)?,
        line_feed: read_u8(section, 0x09)?,
        alter_char_idx: read_u16(section, 0x0A, header.order)?,
        default_left: read_u8(section, 0x0C)?,
        default_glyph_width: read_u8(section, 0x0D)?,
        default_char_width: read_u8(section, 0x0E)?,
        encoding: read_u8(section, 0x0F)?,
        height: read_u8(section, 0x1C)?,
        width: read_u8(section, 0x1D)?,
        ascent: read_u8(section, 0x1E)?,
        reserved: read_u8(section, 0x1F)?,
      },
      tglp_offset: read_u32(section, 0x10, header.order)?,
      cwdh_offset: read_u32(section, 0x14, header.order)?,
      cmap_offset: read_u32(section, 0x18, header.order)?,
    })
  }
}

fn tglp_extent(data: &[u8], header: &Header, finf: &Finf) -> Result<(usize, usize), CodecError> {
  let data_offset = usize::try_from(finf.tglp_offset)
    .map_err(|_| CodecError::Message("BCFNT TGLP offset is impossibly large".to_owned()))?;
  let start = data_offset
    .checked_sub(8)
    .ok_or_else(|| CodecError::Message("BCFNT TGLP offset points before the file".to_owned()))?;
  let section = data
    .get(start..start + TGLP_SIZE)
    .ok_or_else(|| CodecError::Message("BCFNT TGLP header is truncated".to_owned()))?;

  if field::<4>(section, 0)? != *b"TGLP" {
    return Err(CodecError::Message(
      "Not a BCFNT font: the TGLP magic is missing".to_owned(),
    ));
  }

  let section_size = usize::try_from(read_u32(section, 4, header.order)?)
    .map_err(|_| CodecError::Message("BCFNT TGLP size is impossibly large".to_owned()))?;
  if section_size < TGLP_SIZE {
    return Err(CodecError::Message(format!(
      "Invalid BCFNT TGLP size: {section_size} (minimum: {TGLP_SIZE})"
    )));
  }

  let end = start
    .checked_add(section_size)
    .filter(|end| *end <= data.len())
    .ok_or_else(|| CodecError::Message("BCFNT TGLP section is truncated".to_owned()))?;

  let sheet_size = usize::try_from(read_u32(section, 0x0C, header.order)?)
    .map_err(|_| CodecError::Message("BCFNT sheet size is impossibly large".to_owned()))?;
  let sheet_count = usize::from(read_u16(section, 0x10, header.order)?);
  TextureFormat::from_code(read_u16(section, 0x12, header.order)?)
    .ok_or_else(|| CodecError::Message("Unknown BCFNT texture format".to_owned()))?;
  let width = read_u16(section, 0x18, header.order)?;
  let height = read_u16(section, 0x1A, header.order)?;
  if sheet_count > 0 && (sheet_size == 0 || width == 0 || height == 0) {
    return Err(CodecError::Message(
      "BCFNT TGLP describes texture sheets with a zero size or dimension".to_owned(),
    ));
  }

  let sheet_data_offset = usize::try_from(read_u32(section, 0x1C, header.order)?)
    .map_err(|_| CodecError::Message("BCFNT TGLP data offset is impossibly large".to_owned()))?;

  if sheet_data_offset < start + TGLP_SIZE || sheet_data_offset > end {
    return Err(CodecError::Message(
      "BCFNT TGLP data offset is outside its section".to_owned(),
    ));
  }

  sheet_size
    .checked_mul(sheet_count)
    .and_then(|bytes| sheet_data_offset.checked_add(bytes))
    .filter(|sheets_end| *sheets_end <= end)
    .ok_or_else(|| CodecError::Message("BCFNT texture sheets are truncated".to_owned()))?;

  Ok((start, section_size))
}

struct Tglp {
  cell_width: u8,
  cell_height: u8,
  baseline: u8,
  max_char_width: u8,
  format: TextureFormat,
  cols: u16,
  rows: u16,
  width: u16,
  height: u16,
  end: usize,
  gap: Vec<u8>,
  trailing: Vec<u8>,
  /// One artifact per sheet: either a PNG or the native bytes.
  sheets: Vec<Vec<u8>>,
}

impl Tglp {
  fn parse(
    data: &[u8],
    header: &Header,
    finf: &Finf,
    decode_sheets: bool,
  ) -> Result<Self, CodecError> {
    let data_offset = usize::try_from(finf.tglp_offset)
      .map_err(|_| CodecError::Message("BCFNT TGLP offset is impossibly large".to_owned()))?;
    let start = data_offset
      .checked_sub(8)
      .ok_or_else(|| CodecError::Message("BCFNT TGLP offset points before the file".to_owned()))?;

    if start < TGLP_OFFSET {
      return Err(CodecError::Message(format!(
        "BCFNT TGLP section starts at {start:#x}, before the first legal offset {TGLP_OFFSET:#x}"
      )));
    }

    let section = data
      .get(start..start + TGLP_SIZE)
      .ok_or_else(|| CodecError::Message("BCFNT TGLP header is truncated".to_owned()))?;

    if field::<4>(section, 0)? != *b"TGLP" {
      return Err(CodecError::Message(
        "Not a BCFNT font: the TGLP magic is missing".to_owned(),
      ));
    }

    let section_size = usize::try_from(read_u32(section, 4, header.order)?)
      .map_err(|_| CodecError::Message("BCFNT TGLP size is impossibly large".to_owned()))?;
    if section_size < TGLP_SIZE {
      return Err(CodecError::Message(format!(
        "Invalid BCFNT TGLP size: {section_size} (minimum: {TGLP_SIZE})"
      )));
    }
    let end = start
      .checked_add(section_size)
      .filter(|end| *end <= data.len())
      .ok_or_else(|| CodecError::Message("BCFNT TGLP section is truncated".to_owned()))?;

    let sheet_size_field = read_u32(section, 0x0C, header.order)?;
    let sheet_count = read_u16(section, 0x10, header.order)?;
    let format_code = read_u16(section, 0x12, header.order)?;
    let format = TextureFormat::from_code(format_code).ok_or_else(|| {
      CodecError::Message(format!("Unknown BCFNT texture format {format_code:#x}"))
    })?;
    let sheet_data_offset = usize::try_from(read_u32(section, 0x1C, header.order)?)
      .map_err(|_| CodecError::Message("BCFNT TGLP data offset is impossibly large".to_owned()))?;

    if sheet_data_offset < start + TGLP_SIZE || sheet_data_offset > end {
      return Err(CodecError::Message(format!(
        "BCFNT TGLP data offset {sheet_data_offset:#x} is outside the section at {start:#x}..{end:#x}"
      )));
    }

    let sheet_size_field = usize::try_from(sheet_size_field)
      .map_err(|_| CodecError::Message("BCFNT sheet size is impossibly large".to_owned()))?;
    let sheet_bytes = sheet_size_field
      .checked_mul(usize::from(sheet_count))
      .ok_or_else(|| CodecError::Message("BCFNT texture sheets overflow".to_owned()))?;
    let sheets_end = sheet_data_offset
      .checked_add(sheet_bytes)
      .filter(|sheets_end| *sheets_end <= end)
      .ok_or_else(|| CodecError::Message("BCFNT texture sheets are truncated".to_owned()))?;

    let width = read_u16(section, 0x18, header.order)?;
    let height = read_u16(section, 0x1A, header.order)?;

    if sheet_count > 0 {
      ensure_expanded_image_size(width, height, usize::from(sheet_count))?;
      let expected = sheet_size(format, width, height)?;
      if sheet_size_field != expected {
        return Err(CodecError::Message(format!(
          "BCFNT TGLP sheet size is {sheet_size_field} bytes, but {format:?} at {width}x{height} needs {expected}"
        )));
      }
    }

    let gap = data
      .get(start + TGLP_SIZE..sheet_data_offset)
      .ok_or_else(|| CodecError::Message("BCFNT TGLP padding is truncated".to_owned()))?;
    let gap = if decode_sheets {
      gap.to_vec()
    } else {
      Vec::new()
    };

    let png = format.has_png_artifact(width, height);
    let mut sheets = if decode_sheets {
      Vec::with_capacity(usize::from(sheet_count))
    } else {
      Vec::new()
    };

    for index in 0..usize::from(sheet_count) {
      let offset =
        sheet_data_offset
          .checked_add(index.checked_mul(sheet_size_field).ok_or_else(|| {
            CodecError::Message("BCFNT texture sheet offset overflows".to_owned())
          })?)
          .ok_or_else(|| CodecError::Message("BCFNT texture sheet offset overflows".to_owned()))?;
      let sheet = data
        .get(offset..offset + sheet_size_field)
        .ok_or_else(|| CodecError::Message("BCFNT texture sheet is truncated".to_owned()))?;

      if decode_sheets {
        sheets.push(if png {
          let decoded = decode_sheet(sheet, width, height, format, header.order)?;
          sheet_to_png(&decoded)?
        } else {
          sheet.to_vec()
        });
      }
    }

    let trailing = data
      .get(sheets_end..end)
      .ok_or_else(|| CodecError::Message("BCFNT TGLP trailing bytes are truncated".to_owned()))?;
    let trailing = if decode_sheets {
      trailing.to_vec()
    } else {
      Vec::new()
    };

    Ok(Self {
      cell_width: read_u8(section, 0x08)?,
      cell_height: read_u8(section, 0x09)?,
      baseline: read_u8(section, 0x0A)?,
      max_char_width: read_u8(section, 0x0B)?,
      format,
      cols: read_u16(section, 0x14, header.order)?,
      rows: read_u16(section, 0x16, header.order)?,
      width,
      height,
      end,
      gap,
      trailing,
      sheets,
    })
  }

  fn texture_info(&self) -> Result<TextureInfo, CodecError> {
    Ok(TextureInfo {
      cell_width: self.cell_width,
      cell_height: self.cell_height,
      baseline: self.baseline,
      max_char_width: self.max_char_width,
      format: self.format,
      cols: self.cols,
      rows: self.rows,
      width: self.width,
      height: self.height,
    })
  }
}

fn validate_complete(data: &[u8], header: &Header, finf: &Finf) -> Result<(), CodecError> {
  validate_file_size(data.len())?;
  let tglp = Tglp::parse(data, header, finf, false)?;
  let (cwdh, _) = parse_cwdh_chain(data, header.order, finf.cwdh_offset, tglp.end)?;
  let (cmap, _) = parse_cmap_chain(data, header.order, finf.cmap_offset, tglp.end)?;

  let section_count = 2usize
    .checked_add(cwdh.sections.len())
    .and_then(|count| count.checked_add(cmap.sections.len()))
    .ok_or_else(|| CodecError::Message("BCFNT section count overflows".to_owned()))?;
  if usize::try_from(header.sections).ok() != Some(section_count) {
    return Err(CodecError::Message(format!(
      "BCFNT header declares {} sections, but the file holds {section_count}",
      header.sections
    )));
  }
  Ok(())
}

fn validate_file_size(size: usize) -> Result<(), CodecError> {
  if size > MAX_FILE_SIZE {
    return Err(CodecError::Message(format!(
      "BCFNT input is {size} bytes; maximum supported size is {MAX_FILE_SIZE} bytes"
    )));
  }
  Ok(())
}

fn ensure_expanded_image_size(
  width: u16,
  height: u16,
  sheet_count: usize,
) -> Result<(), CodecError> {
  let expanded = usize::from(width)
    .checked_mul(usize::from(height))
    .and_then(|pixels| pixels.checked_mul(4))
    .and_then(|bytes| bytes.checked_mul(sheet_count))
    .ok_or_else(|| CodecError::Message("BCFNT expanded image size overflows".to_owned()))?;
  if expanded > MAX_EXPANDED_IMAGE_SIZE {
    return Err(CodecError::Message(format!(
      "BCFNT texture sheets expand to {expanded} RGBA bytes; maximum supported size is {MAX_EXPANDED_IMAGE_SIZE} bytes"
    )));
  }
  Ok(())
}

fn parse_cwdh_chain(
  data: &[u8],
  order: ByteOrder,
  first: u32,
  after: usize,
) -> Result<(CwdhMetadata, usize), CodecError> {
  let mut sections = Vec::new();
  let mut next = usize::try_from(first)
    .map_err(|_| CodecError::Message("BCFNT CWDH offset is impossibly large".to_owned()))?;
  let mut previous_end = after;
  let mut visited = HashSet::new();

  while next != 0 {
    if !visited.insert(next) {
      return Err(CodecError::Message(
        "BCFNT CWDH chain contains a loop".to_owned(),
      ));
    }

    let start = next
      .checked_sub(8)
      .ok_or_else(|| CodecError::Message("BCFNT CWDH offset points before the file".to_owned()))?;

    if start < previous_end {
      return Err(CodecError::Message(format!(
        "BCFNT CWDH section at {start:#x} overlaps or runs backwards"
      )));
    }

    let header = data
      .get(start..start + CWDH_SIZE)
      .ok_or_else(|| CodecError::Message("BCFNT CWDH header is truncated".to_owned()))?;

    if field::<4>(header, 0)? != *b"CWDH" {
      return Err(CodecError::Message(
        "Invalid BCFNT CWDH magic bytes".to_owned(),
      ));
    }

    let section_size = usize::try_from(read_u32(header, 4, order)?)
      .map_err(|_| CodecError::Message("BCFNT CWDH size is impossibly large".to_owned()))?;
    if section_size < CWDH_SIZE {
      return Err(CodecError::Message(format!(
        "Invalid BCFNT CWDH size: {section_size} (minimum: {CWDH_SIZE})"
      )));
    }
    let end = start
      .checked_add(section_size)
      .filter(|end| *end <= data.len())
      .ok_or_else(|| CodecError::Message("BCFNT CWDH section is truncated".to_owned()))?;

    let begin_index = read_u16(header, 8, order)?;
    let end_index = read_u16(header, 0x0A, order)?;
    if end_index < begin_index {
      return Err(CodecError::Message(format!(
        "BCFNT CWDH section ends at index {end_index}, before its start index {begin_index}"
      )));
    }

    let count = usize::from(end_index - begin_index) + 1;
    let entries_bytes = count
      .checked_mul(3)
      .ok_or_else(|| CodecError::Message("BCFNT CWDH entries overflow".to_owned()))?;
    let payload = data
      .get(start + CWDH_SIZE..end)
      .ok_or_else(|| CodecError::Message("BCFNT CWDH payload is truncated".to_owned()))?;

    if payload.len() < entries_bytes {
      return Err(CodecError::Message(format!(
        "BCFNT CWDH section is {} bytes, too small for {count} entries",
        payload.len()
      )));
    }

    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
      let offset = index * 3;
      let left = *payload
        .get(offset)
        .ok_or_else(|| CodecError::Message("BCFNT CWDH entry is truncated".to_owned()))?
        as i8;
      let glyph = *payload
        .get(offset + 1)
        .ok_or_else(|| CodecError::Message("BCFNT CWDH entry is truncated".to_owned()))?;
      let char_width = *payload
        .get(offset + 2)
        .ok_or_else(|| CodecError::Message("BCFNT CWDH entry is truncated".to_owned()))?;

      entries.push(CwdhEntry {
        left,
        glyph,
        char_width,
      });
    }

    let padding = payload
      .get(entries_bytes..)
      .ok_or_else(|| CodecError::Message("BCFNT CWDH padding is truncated".to_owned()))?;

    sections.push(CwdhSection {
      start: begin_index,
      end: end_index,
      entries,
      padding: encode_hex(padding),
    });

    previous_end = end;
    next = usize::try_from(read_u32(header, 0x0C, order)?)
      .map_err(|_| CodecError::Message("BCFNT CWDH offset is impossibly large".to_owned()))?;
  }

  Ok((CwdhMetadata { sections }, previous_end))
}

fn parse_cmap_chain(
  data: &[u8],
  order: ByteOrder,
  first: u32,
  after: usize,
) -> Result<(CmapMetadata, usize), CodecError> {
  let mut sections = Vec::new();
  let mut next = usize::try_from(first)
    .map_err(|_| CodecError::Message("BCFNT CMAP offset is impossibly large".to_owned()))?;
  let mut previous_end = after;
  let mut visited = HashSet::new();

  while next != 0 {
    if !visited.insert(next) {
      return Err(CodecError::Message(
        "BCFNT CMAP chain contains a loop".to_owned(),
      ));
    }

    let start = next
      .checked_sub(8)
      .ok_or_else(|| CodecError::Message("BCFNT CMAP offset points before the file".to_owned()))?;

    if start < previous_end {
      return Err(CodecError::Message(format!(
        "BCFNT CMAP section at {start:#x} overlaps or runs backwards"
      )));
    }

    let header = data
      .get(start..start + CMAP_SIZE)
      .ok_or_else(|| CodecError::Message("BCFNT CMAP header is truncated".to_owned()))?;

    if field::<4>(header, 0)? != *b"CMAP" {
      return Err(CodecError::Message(
        "Invalid BCFNT CMAP magic bytes".to_owned(),
      ));
    }

    let section_size = usize::try_from(read_u32(header, 4, order)?)
      .map_err(|_| CodecError::Message("BCFNT CMAP size is impossibly large".to_owned()))?;
    if section_size < CMAP_SIZE {
      return Err(CodecError::Message(format!(
        "Invalid BCFNT CMAP size: {section_size} (minimum: {CMAP_SIZE})"
      )));
    }
    let end = start
      .checked_add(section_size)
      .filter(|end| *end <= data.len())
      .ok_or_else(|| CodecError::Message("BCFNT CMAP section is truncated".to_owned()))?;

    let begin_code = read_u16(header, 8, order)?;
    let end_code = read_u16(header, 0x0A, order)?;
    let method = read_u16(header, 0x0C, order)?;
    let unknown = read_u16(header, 0x0E, order)?;
    let payload = data
      .get(start + CMAP_SIZE..end)
      .ok_or_else(|| CodecError::Message("BCFNT CMAP payload is truncated".to_owned()))?;

    let (section, consumed) = match method {
      0 => {
        let index_offset = read_u16(payload, 0, order)?;
        (
          CmapSection::Direct {
            start: begin_code,
            end: end_code,
            unknown,
            index_offset,
            padding: String::new(),
          },
          2usize,
        )
      }
      1 => {
        if end_code < begin_code {
          return Err(CodecError::Message(format!(
            "BCFNT CMAP section ends at code {end_code:#x}, before its start code {begin_code:#x}"
          )));
        }

        let count = usize::from(end_code - begin_code) + 1;
        let bytes = count
          .checked_mul(2)
          .ok_or_else(|| CodecError::Message("BCFNT CMAP table overflows".to_owned()))?;
        if payload.len() < bytes {
          return Err(CodecError::Message(format!(
            "BCFNT CMAP section is {} bytes, too small for a {count}-entry table",
            payload.len()
          )));
        }

        let mut index_table = Vec::with_capacity(count);
        for index in 0..count {
          index_table.push(read_u16(payload, index * 2, order)?);
        }

        (
          CmapSection::Table {
            start: begin_code,
            end: end_code,
            unknown,
            index_table,
            padding: String::new(),
          },
          bytes,
        )
      }
      2 => {
        let count = usize::from(read_u16(payload, 0, order)?);
        let bytes = count
          .checked_mul(4)
          .and_then(|bytes| bytes.checked_add(2))
          .ok_or_else(|| CodecError::Message("BCFNT CMAP scan table overflows".to_owned()))?;
        if payload.len() < bytes {
          return Err(CodecError::Message(format!(
            "BCFNT CMAP section is {} bytes, too small for {count} scan entries",
            payload.len()
          )));
        }

        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
          let offset = 2 + index * 4;
          entries.push(CmapEntry {
            code: read_u16(payload, offset, order)?,
            glyph: read_u16(payload, offset + 2, order)?,
          });
        }

        (
          CmapSection::Scan {
            start: begin_code,
            end: end_code,
            unknown,
            entries,
            padding: String::new(),
          },
          bytes,
        )
      }
      _ => {
        return Err(CodecError::Message(format!(
          "Unknown BCFNT CMAP mapping method {method}"
        )));
      }
    };

    let padding = payload
      .get(consumed..)
      .ok_or_else(|| CodecError::Message("BCFNT CMAP padding is truncated".to_owned()))?;
    let section = with_cmap_padding(section, encode_hex(padding));

    sections.push(section);
    previous_end = end;
    next = usize::try_from(read_u32(header, 0x10, order)?)
      .map_err(|_| CodecError::Message("BCFNT CMAP offset is impossibly large".to_owned()))?;
  }

  Ok((CmapMetadata { sections }, previous_end))
}

fn with_cmap_padding(section: CmapSection, padding: String) -> CmapSection {
  match section {
    CmapSection::Direct {
      start,
      end,
      unknown,
      index_offset,
      ..
    } => CmapSection::Direct {
      start,
      end,
      unknown,
      index_offset,
      padding,
    },
    CmapSection::Table {
      start,
      end,
      unknown,
      index_table,
      ..
    } => CmapSection::Table {
      start,
      end,
      unknown,
      index_table,
      padding,
    },
    CmapSection::Scan {
      start,
      end,
      unknown,
      entries,
      ..
    } => CmapSection::Scan {
      start,
      end,
      unknown,
      entries,
      padding,
    },
  }
}
enum TextureArtifact<'a> {
  Png(&'a [u8]),
  Raw(&'a [u8]),
}

struct SortedArtifacts<'a> {
  metadata: &'a [u8],
  cwdh: &'a [u8],
  cmap: &'a [u8],
  textures: BTreeMap<usize, TextureArtifact<'a>>,
}

fn sort_artifacts(artifacts: &[DecodedArtifact]) -> Result<SortedArtifacts<'_>, CodecError> {
  let mut metadata = None;
  let mut cwdh = None;
  let mut cmap = None;
  let mut textures = BTreeMap::new();

  for artifact in artifacts {
    let bytes = artifact.data.as_slice();
    match artifact.name.as_str() {
      metadata::METADATA_NAME => {
        if metadata.replace(bytes).is_some() {
          return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
        }
      }
      metadata::CWDH_NAME => {
        if cwdh.replace(bytes).is_some() {
          return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
        }
      }
      metadata::CMAP_NAME => {
        if cmap.replace(bytes).is_some() {
          return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
        }
      }
      name => {
        let Some((index, png)) = texture_artifact(name) else {
          return Err(CodecError::Message(format!("Unexpected artifact: {name}")));
        };

        let texture = if png {
          TextureArtifact::Png(bytes)
        } else {
          TextureArtifact::Raw(bytes)
        };

        if textures.insert(index, texture).is_some() {
          return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
        }
      }
    }
  }

  Ok(SortedArtifacts {
    metadata: metadata.ok_or_else(|| {
      CodecError::Message(format!("Expected a {} artifact", metadata::METADATA_NAME))
    })?,
    cwdh: cwdh
      .ok_or_else(|| CodecError::Message(format!("Expected a {} artifact", metadata::CWDH_NAME)))?,
    cmap: cmap
      .ok_or_else(|| CodecError::Message(format!("Expected a {} artifact", metadata::CMAP_NAME)))?,
    textures,
  })
}

fn build_sheets(
  texture: &TextureInfo,
  order: ByteOrder,
  textures: &BTreeMap<usize, TextureArtifact<'_>>,
) -> Result<Vec<Vec<u8>>, CodecError> {
  let mut sheets = Vec::with_capacity(textures.len());
  let png_expected = texture
    .format
    .has_png_artifact(texture.width, texture.height);
  let native_size = if textures.is_empty() {
    0
  } else {
    sheet_size(texture.format, texture.width, texture.height)?
  };

  for index in 0..textures.len() {
    let name = texture_name(index, png_expected);
    let artifact = textures
      .get(&index)
      .ok_or_else(|| CodecError::Message(format!("Expected a {name} artifact, found none")))?;

    let sheet = match (artifact, png_expected) {
      (TextureArtifact::Png(data), true) => {
        let decoded = png_to_sheet(data, texture.width, texture.height)?;
        let encoded = encode_sheet(&decoded, texture.format, order)?;

        if encoded.len() != native_size {
          return Err(CodecError::Message(format!(
            "{name} encodes to {} bytes, expected {native_size}",
            encoded.len()
          )));
        }

        encoded
      }
      (TextureArtifact::Raw(data), false) => {
        if data.len() != native_size {
          return Err(CodecError::Message(format!(
            "{name} is {} bytes, expected {native_size}",
            data.len()
          )));
        }

        data.to_vec()
      }
      (TextureArtifact::Png(_), false) => {
        return Err(CodecError::Message(format!(
          "{} is not supported for {:?} texture sheets; expected {name}",
          texture_name(index, true),
          texture.format
        )));
      }
      (TextureArtifact::Raw(_), true) => {
        return Err(CodecError::Message(format!(
          "{} is not an editable {:?} texture; expected {name}",
          texture_name(index, false),
          texture.format
        )));
      }
    };

    sheets.push(sheet);
  }

  Ok(sheets)
}

fn encode_cwdh_sections(
  sections: &[CwdhSection],
  start: usize,
  order: ByteOrder,
) -> Result<Vec<(usize, usize, EncodedSection)>, CodecError> {
  let mut encoded = Vec::with_capacity(sections.len());
  let mut position = start;

  for section in sections {
    let bytes = encode_cwdh(section, order)?;
    let end = position
      .checked_add(bytes.len())
      .ok_or_else(|| CodecError::Message("BCFNT CWDH section overflows".to_owned()))?;
    encoded.push((position, end, EncodedSection { bytes }));
    position = end;
  }

  for index in 0..encoded.len() {
    if let Some(next) = encoded.get(index + 1).map(|(position, _, _)| *position) {
      let next_offset = u32::try_from(next + 8)
        .map_err(|_| CodecError::Message("BCFNT CWDH offset is too large".to_owned()))?;
      let bytes = &mut encoded[index].2.bytes;
      put_u32(bytes, 0x0C, next_offset, order)?;
    }
  }

  Ok(encoded)
}

fn encode_cmap_sections(
  sections: &[CmapSection],
  start: usize,
  order: ByteOrder,
) -> Result<Vec<(usize, usize, EncodedSection)>, CodecError> {
  let mut encoded = Vec::with_capacity(sections.len());
  let mut position = start;

  for section in sections {
    let bytes = encode_cmap(section, order)?;
    let end = position
      .checked_add(bytes.len())
      .ok_or_else(|| CodecError::Message("BCFNT CMAP section overflows".to_owned()))?;
    encoded.push((position, end, EncodedSection { bytes }));
    position = end;
  }

  for index in 0..encoded.len() {
    if let Some(next) = encoded.get(index + 1).map(|(position, _, _)| *position) {
      let next_offset = u32::try_from(next + 8)
        .map_err(|_| CodecError::Message("BCFNT CMAP offset is too large".to_owned()))?;
      let bytes = &mut encoded[index].2.bytes;
      put_u32(bytes, 0x10, next_offset, order)?;
    }
  }

  Ok(encoded)
}

struct EncodedSection {
  bytes: Vec<u8>,
}

fn encode_cwdh(section: &CwdhSection, order: ByteOrder) -> Result<Vec<u8>, CodecError> {
  if section.end < section.start {
    return Err(CodecError::Message(format!(
      "BCFNT CWDH section ends at index {}, before its start index {}",
      section.end, section.start
    )));
  }

  let count = usize::from(section.end - section.start) + 1;
  if section.entries.len() != count {
    return Err(CodecError::Message(format!(
      "BCFNT CWDH section has {} entries, expected {count}",
      section.entries.len()
    )));
  }

  let padding = decode_hex("CWDH padding", &section.padding)?;
  let size = CWDH_SIZE
    .checked_add(
      count
        .checked_mul(3)
        .ok_or_else(|| CodecError::Message("BCFNT CWDH entries overflow".to_owned()))?,
    )
    .and_then(|size| size.checked_add(padding.len()))
    .ok_or_else(|| CodecError::Message("BCFNT CWDH section overflows".to_owned()))?;
  let size_u32 = u32::try_from(size)
    .map_err(|_| CodecError::Message("BCFNT CWDH section is too large".to_owned()))?;

  let mut out = Vec::with_capacity(size);
  out.extend_from_slice(b"CWDH");
  write_u32(&mut out, size_u32, order);
  write_u16(&mut out, section.start, order);
  write_u16(&mut out, section.end, order);
  write_u32(&mut out, 0, order);

  for entry in &section.entries {
    out.push(entry.left as u8);
    out.push(entry.glyph);
    out.push(entry.char_width);
  }

  out.extend_from_slice(&padding);

  Ok(out)
}

fn encode_cmap(section: &CmapSection, order: ByteOrder) -> Result<Vec<u8>, CodecError> {
  let padding = decode_hex("CMAP padding", section.padding())?;
  let mut payload = Vec::new();

  match section {
    CmapSection::Direct {
      start,
      end,
      index_offset,
      ..
    } => {
      if end < start {
        return Err(CodecError::Message(format!(
          "BCFNT CMAP section ends at code {end:#x}, before its start code {start:#x}"
        )));
      }
      write_u16(&mut payload, *index_offset, order);
    }
    CmapSection::Table {
      start,
      end,
      index_table,
      ..
    } => {
      if end < start {
        return Err(CodecError::Message(format!(
          "BCFNT CMAP section ends at code {end:#x}, before its start code {start:#x}"
        )));
      }
      let count = usize::from(*end - *start) + 1;
      if index_table.len() != count {
        return Err(CodecError::Message(format!(
          "BCFNT CMAP table has {} entries, expected {count}",
          index_table.len()
        )));
      }
      for index in index_table {
        write_u16(&mut payload, *index, order);
      }
    }
    CmapSection::Scan { entries, .. } => {
      let count = u16::try_from(entries.len()).map_err(|_| {
        CodecError::Message("BCFNT CMAP scan table has too many entries".to_owned())
      })?;

      let mut sorted: Vec<CmapEntry> = entries.clone();
      sorted.sort_by_key(|entry| entry.code);
      for pair in sorted.windows(2) {
        if pair[0].code == pair[1].code {
          return Err(CodecError::Message(format!(
            "BCFNT CMAP scan table maps code {:#x} twice",
            pair[0].code
          )));
        }
      }

      write_u16(&mut payload, count, order);
      for entry in sorted {
        write_u16(&mut payload, entry.code, order);
        write_u16(&mut payload, entry.glyph, order);
      }
    }
  }

  let size = CMAP_SIZE
    .checked_add(payload.len())
    .and_then(|size| size.checked_add(padding.len()))
    .ok_or_else(|| CodecError::Message("BCFNT CMAP section overflows".to_owned()))?;
  let size_u32 = u32::try_from(size)
    .map_err(|_| CodecError::Message("BCFNT CMAP section is too large".to_owned()))?;

  let mut out = Vec::with_capacity(size);
  out.extend_from_slice(b"CMAP");
  write_u32(&mut out, size_u32, order);
  write_u16(&mut out, section.start(), order);
  write_u16(&mut out, section.end(), order);
  write_u16(&mut out, section.method(), order);
  match section {
    CmapSection::Direct { unknown, .. }
    | CmapSection::Table { unknown, .. }
    | CmapSection::Scan { unknown, .. } => write_u16(&mut out, *unknown, order),
  }
  write_u32(&mut out, 0, order);
  out.extend_from_slice(&payload);
  out.extend_from_slice(&padding);

  Ok(out)
}

fn write_header(
  out: &mut Vec<u8>,
  metadata: &BcfntMetadata,
  file_size: u32,
  section_count: u32,
) -> Result<(), CodecError> {
  out.extend_from_slice(&metadata.magic.bytes());
  match metadata.byte_order {
    ByteOrder::Little => out.extend_from_slice(&[0xFF, 0xFE]),
    ByteOrder::Big => out.extend_from_slice(&[0xFE, 0xFF]),
  }
  write_u16(out, HEADER_SIZE as u16, metadata.byte_order);
  write_u32(out, metadata.version, metadata.byte_order);
  write_u32(out, file_size, metadata.byte_order);
  write_u32(out, section_count, metadata.byte_order);
  Ok(())
}

fn write_finf(
  out: &mut Vec<u8>,
  font: &FontInfo,
  tglp_offset: u32,
  cwdh_offset: u32,
  cmap_offset: u32,
  order: ByteOrder,
) -> Result<(), CodecError> {
  out.extend_from_slice(b"FINF");
  write_u32(out, FINF_SIZE as u32, order);
  out.push(font.font_type);
  out.push(font.line_feed);
  write_u16(out, font.alter_char_idx, order);
  out.push(font.default_left);
  out.push(font.default_glyph_width);
  out.push(font.default_char_width);
  out.push(font.encoding);
  write_u32(out, tglp_offset, order);
  write_u32(out, cwdh_offset, order);
  write_u32(out, cmap_offset, order);
  out.push(font.height);
  out.push(font.width);
  out.push(font.ascent);
  out.push(font.reserved);
  Ok(())
}

/// The TGLP header stores the sheet data offset explicitly, and the format
/// documentation imposes no alignment on it. The encoder therefore places the
/// sheets at the earliest valid offset: directly after the TGLP header. With
/// the fixed headers here that offset is 0x54, so a canonical font needs no gap
/// bytes at all. Decoding still accepts any in-section offset whose gap bytes
/// are zero.
fn tglp_data_offset() -> Result<usize, CodecError> {
  TGLP_OFFSET
    .checked_add(TGLP_SIZE)
    .ok_or_else(|| CodecError::Message("BCFNT TGLP header offset overflows".to_owned()))
}

fn write_tglp_header(
  out: &mut Vec<u8>,
  texture: &TextureInfo,
  sheet_size: usize,
  sheet_count: usize,
  section_size: usize,
  data_offset: usize,
  order: ByteOrder,
) -> Result<(), CodecError> {
  let section_size = u32::try_from(section_size)
    .map_err(|_| CodecError::Message("BCFNT TGLP section is too large".to_owned()))?;
  let sheet_size = u32::try_from(sheet_size)
    .map_err(|_| CodecError::Message("BCFNT sheet size is too large".to_owned()))?;
  let sheet_count = u16::try_from(sheet_count)
    .map_err(|_| CodecError::Message("BCFNT has too many texture sheets".to_owned()))?;
  let data_offset = u32::try_from(data_offset)
    .map_err(|_| CodecError::Message("BCFNT TGLP data offset is too large".to_owned()))?;

  out.extend_from_slice(b"TGLP");
  write_u32(out, section_size, order);
  out.push(texture.cell_width);
  out.push(texture.cell_height);
  out.push(texture.baseline);
  out.push(texture.max_char_width);
  write_u32(out, sheet_size, order);
  write_u16(out, sheet_count, order);
  write_u16(out, texture.format.code(), order);
  write_u16(out, texture.cols, order);
  write_u16(out, texture.rows, order);
  write_u16(out, texture.width, order);
  write_u16(out, texture.height, order);
  write_u32(out, data_offset, order);
  Ok(())
}

fn read_u8(data: &[u8], offset: usize) -> Result<u8, CodecError> {
  data
    .get(offset)
    .copied()
    .ok_or_else(|| CodecError::Message(format!("BCFNT structure is truncated at {offset:#x}")))
}

fn read_u16(data: &[u8], offset: usize, order: ByteOrder) -> Result<u16, CodecError> {
  let bytes = field::<2>(data, offset)?;
  Ok(match order {
    ByteOrder::Little => u16::from_le_bytes(bytes),
    ByteOrder::Big => u16::from_be_bytes(bytes),
  })
}

fn read_u32(data: &[u8], offset: usize, order: ByteOrder) -> Result<u32, CodecError> {
  let bytes = field::<4>(data, offset)?;
  Ok(match order {
    ByteOrder::Little => u32::from_le_bytes(bytes),
    ByteOrder::Big => u32::from_be_bytes(bytes),
  })
}

fn write_u16(out: &mut Vec<u8>, value: u16, order: ByteOrder) {
  let bytes = match order {
    ByteOrder::Little => value.to_le_bytes(),
    ByteOrder::Big => value.to_be_bytes(),
  };
  out.extend_from_slice(&bytes);
}

fn write_u32(out: &mut Vec<u8>, value: u32, order: ByteOrder) {
  let bytes = match order {
    ByteOrder::Little => value.to_le_bytes(),
    ByteOrder::Big => value.to_be_bytes(),
  };
  out.extend_from_slice(&bytes);
}

fn put_u32(data: &mut [u8], offset: usize, value: u32, order: ByteOrder) -> Result<(), CodecError> {
  let target = data
    .get_mut(offset..offset + 4)
    .ok_or_else(|| CodecError::Message("BCFNT section is too small".to_owned()))?;
  let bytes = match order {
    ByteOrder::Little => value.to_le_bytes(),
    ByteOrder::Big => value.to_be_bytes(),
  };
  target.copy_from_slice(&bytes);

  Ok(())
}
