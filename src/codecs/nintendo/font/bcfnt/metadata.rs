//! Metadata artifacts for a BCFNT font.
//!
//! The binary sections that describe the font are kept as readable JSON so a
//! project can edit them, while each texture sheet is a single separate image
//! artifact. Serialization details that can be recalculated (offsets, sheet
//! sizes, padding) are deliberately absent.

use crate::codecs::errors::CodecError;
use serde::{Deserialize, Serialize};

pub(crate) const METADATA_NAME: &str = "BCFNT.json";
pub(crate) const CWDH_NAME: &str = "CWDH.json";
pub(crate) const CMAP_NAME: &str = "CMAP.json";

/// The single artifact name for a texture sheet.
///
/// `png` selects the editable image form; otherwise the sheet stays in its
/// native format because it cannot be represented faithfully as editable
/// pixels.
pub(crate) fn texture_name(index: usize, png: bool) -> String {
  format!("texture{index}.{}", if png { "png" } else { "bin" })
}

/// Splits a texture artifact name into its sheet index and whether it is a PNG.
pub(crate) fn texture_artifact(name: &str) -> Option<(usize, bool)> {
  let (number, extension) = name.strip_prefix("texture")?.split_once('.')?;
  let index = number.parse().ok()?;

  match extension {
    "png" => Some((index, true)),
    "bin" => Some((index, false)),
    _ => None,
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FontMagic {
  #[serde(rename = "CFNT")]
  Cfnt,
  #[serde(rename = "CFNU")]
  Cfnu,
}

impl FontMagic {
  pub(crate) fn bytes(self) -> [u8; 4] {
    match self {
      Self::Cfnt => *b"CFNT",
      Self::Cfnu => *b"CFNU",
    }
  }

  pub(crate) fn from_bytes(bytes: [u8; 4]) -> Option<Self> {
    match &bytes {
      b"CFNT" => Some(Self::Cfnt),
      b"CFNU" => Some(Self::Cfnu),
      _ => None,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ByteOrder {
  Little,
  Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TextureFormat {
  #[serde(rename = "RGBA8")]
  Rgba8,
  #[serde(rename = "RGB8")]
  Rgb8,
  #[serde(rename = "RGBA5551")]
  Rgba5551,
  #[serde(rename = "RGB565")]
  Rgb565,
  #[serde(rename = "RGBA4")]
  Rgba4,
  #[serde(rename = "LA8")]
  La8,
  #[serde(rename = "HILO8")]
  Hilo8,
  #[serde(rename = "L8")]
  L8,
  #[serde(rename = "A8")]
  A8,
  #[serde(rename = "LA4")]
  La4,
  #[serde(rename = "L4")]
  L4,
  #[serde(rename = "A4")]
  A4,
  #[serde(rename = "ETC1")]
  Etc1,
  #[serde(rename = "ETC1A4")]
  Etc1A4,
}

impl TextureFormat {
  pub(crate) fn from_code(code: u16) -> Option<Self> {
    Some(match code {
      0x00 => Self::Rgba8,
      0x01 => Self::Rgb8,
      0x02 => Self::Rgba5551,
      0x03 => Self::Rgb565,
      0x04 => Self::Rgba4,
      0x05 => Self::La8,
      0x06 => Self::Hilo8,
      0x07 => Self::L8,
      0x08 => Self::A8,
      0x09 => Self::La4,
      0x0A => Self::L4,
      0x0B => Self::A4,
      0x0C => Self::Etc1,
      0x0D => Self::Etc1A4,
      _ => return None,
    })
  }

  pub(crate) fn code(self) -> u16 {
    match self {
      Self::Rgba8 => 0x00,
      Self::Rgb8 => 0x01,
      Self::Rgba5551 => 0x02,
      Self::Rgb565 => 0x03,
      Self::Rgba4 => 0x04,
      Self::La8 => 0x05,
      Self::Hilo8 => 0x06,
      Self::L8 => 0x07,
      Self::A8 => 0x08,
      Self::La4 => 0x09,
      Self::L4 => 0x0A,
      Self::A4 => 0x0B,
      Self::Etc1 => 0x0C,
      Self::Etc1A4 => 0x0D,
    }
  }

  /// Bits used by one pixel of a linear (non-ETC) sheet.
  pub(crate) fn bits_per_pixel(self) -> Option<usize> {
    Some(match self {
      Self::Rgba8 => 32,
      Self::Rgb8 => 24,
      Self::Rgba5551 | Self::Rgb565 | Self::Rgba4 | Self::La8 | Self::Hilo8 => 16,
      Self::L8 | Self::A8 | Self::La4 => 8,
      Self::L4 | Self::A4 => 4,
      Self::Etc1 | Self::Etc1A4 => return None,
    })
  }

  /// Whether a sheet of this format can be represented by an editable PNG.
  ///
  /// HILO8 has no lossless RGBA interpretation, and the swizzled linear
  /// formats need at least one full 8x8 tile.
  pub(crate) fn has_png_artifact(self, width: u16, height: u16) -> bool {
    !matches!(self, Self::Hilo8) && width >= 8 && height >= 8
  }

  pub(crate) fn is_etc(self) -> bool {
    matches!(self, Self::Etc1 | Self::Etc1A4)
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FontInfo {
  pub font_type: u8,
  pub line_feed: u8,
  pub alter_char_idx: u16,
  pub default_left: u8,
  pub default_glyph_width: u8,
  pub default_char_width: u8,
  pub encoding: u8,
  pub height: u8,
  pub width: u8,
  pub ascent: u8,
  pub reserved: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextureInfo {
  pub cell_width: u8,
  pub cell_height: u8,
  pub baseline: u8,
  pub max_char_width: u8,
  pub format: TextureFormat,
  pub cols: u16,
  pub rows: u16,
  pub width: u16,
  pub height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BcfntMetadata {
  pub magic: FontMagic,
  pub byte_order: ByteOrder,
  pub version: u32,
  pub font: FontInfo,
  pub texture: TextureInfo,
  /// Bytes between the TGLP header and the first texture sheet, as hex.
  #[serde(default, skip_serializing_if = "is_none_or_empty")]
  pub tglp_gap: Option<String>,
  /// Bytes after the final texture sheet but inside the TGLP section, as hex.
  #[serde(default, skip_serializing_if = "is_none_or_empty")]
  pub tglp_trailing: Option<String>,
  /// Bytes after all declared sections, as hex.
  #[serde(default, skip_serializing_if = "is_none_or_empty")]
  pub file_trailer: Option<String>,
}

fn is_none_or_empty(value: &Option<String>) -> bool {
  value.as_ref().is_none_or(String::is_empty)
}

impl BcfntMetadata {
  pub(crate) fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the BCFNT metadata: {err}")))
  }

  pub(crate) fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the BCFNT metadata artifact: {err}"))
    })
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CwdhEntry {
  pub left: i8,
  pub glyph: u8,
  #[serde(rename = "char")]
  pub char_width: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CwdhSection {
  pub start: u16,
  pub end: u16,
  pub entries: Vec<CwdhEntry>,
  /// Bytes after the entries and before the end of the CWDH section.
  pub padding: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CwdhMetadata {
  pub sections: Vec<CwdhSection>,
}

impl CwdhMetadata {
  pub(crate) fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the CWDH metadata: {err}")))
  }

  pub(crate) fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the CWDH metadata artifact: {err}"))
    })
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CmapEntry {
  pub code: u16,
  pub glyph: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CmapSection {
  Direct {
    start: u16,
    end: u16,
    unknown: u16,
    index_offset: u16,
    padding: String,
  },
  Table {
    start: u16,
    end: u16,
    unknown: u16,
    index_table: Vec<u16>,
    padding: String,
  },
  Scan {
    start: u16,
    end: u16,
    unknown: u16,
    entries: Vec<CmapEntry>,
    padding: String,
  },
}

impl CmapSection {
  pub(crate) fn start(&self) -> u16 {
    match self {
      Self::Direct { start, .. } | Self::Table { start, .. } | Self::Scan { start, .. } => *start,
    }
  }

  pub(crate) fn end(&self) -> u16 {
    match self {
      Self::Direct { end, .. } | Self::Table { end, .. } | Self::Scan { end, .. } => *end,
    }
  }

  pub(crate) fn method(&self) -> u16 {
    match self {
      Self::Direct { .. } => 0,
      Self::Table { .. } => 1,
      Self::Scan { .. } => 2,
    }
  }

  pub(crate) fn padding(&self) -> &str {
    match self {
      Self::Direct { padding, .. } | Self::Table { padding, .. } | Self::Scan { padding, .. } => {
        padding
      }
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CmapMetadata {
  pub sections: Vec<CmapSection>,
}

impl CmapMetadata {
  pub(crate) fn to_json(&self) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec_pretty(self)
      .map_err(|err| CodecError::Message(format!("Could not write the CMAP metadata: {err}")))
  }

  pub(crate) fn from_json(json: &[u8]) -> Result<Self, CodecError> {
    serde_json::from_slice(json).map_err(|err| {
      CodecError::Message(format!("Could not read the CMAP metadata artifact: {err}"))
    })
  }
}

pub(crate) fn decode_hex(what: &str, value: &str) -> Result<Vec<u8>, CodecError> {
  if value.is_empty() {
    return Ok(Vec::new());
  }

  hex::decode(value)
    .map_err(|err| CodecError::Message(format!("The {what} is not valid hexadecimal: {err}")))
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
  hex::encode(bytes)
}
