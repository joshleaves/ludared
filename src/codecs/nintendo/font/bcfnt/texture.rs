//! Texture-sheet conversion for BCFNT fonts.
//!
//! Sheet pixels are stored in 8x8 tiles, and each tile is further split into
//! 2x2 blocks, 2x2 pixel groups and 2x2 pixels. The conversion here follows
//! that order so the bytes written back into a sheet keep their original
//! meaning.

use super::metadata::{ByteOrder, TextureFormat};
use crate::codecs::errors::CodecError;
use std::io::Cursor;

const ETC_INDIV_RED1_OFFSET: u32 = 60;
const ETC_INDIV_GREEN1_OFFSET: u32 = 52;
const ETC_INDIV_BLUE1_OFFSET: u32 = 44;

const ETC_DIFF_RED1_OFFSET: u32 = 59;
const ETC_DIFF_GREEN1_OFFSET: u32 = 51;
const ETC_DIFF_BLUE_OFFSET: u32 = 43;

const ETC_RED2_OFFSET: u32 = 56;
const ETC_GREEN2_OFFSET: u32 = 48;
const ETC_BLUE_OFFSET: u32 = 40;

const ETC_TABLE1_OFFSET: u32 = 37;
const ETC_TABLE2_OFFSET: u32 = 34;

const ETC_DIFFERENTIAL_BIT: u32 = 33;
const ETC_ORIENTATION_BIT: u32 = 32;

const ETC_MODIFIERS: [[i32; 2]; 8] = [
  [2, 8],
  [5, 17],
  [9, 29],
  [13, 42],
  [18, 60],
  [24, 80],
  [33, 106],
  [47, 183],
];

pub(crate) struct Sheet {
  pub width: usize,
  pub height: usize,
  pub pixels: Vec<[u8; 4]>,
}

impl Sheet {
  fn new(width: u16, height: u16) -> Result<Self, CodecError> {
    let width = usize::from(width);
    let height = usize::from(height);

    if width == 0 || height == 0 {
      return Err(CodecError::Message(
        "BCFNT texture sheet has a zero dimension".to_owned(),
      ));
    }

    super::ensure_expanded_image_size(width as u16, height as u16, 1)?;
    let len = width
      .checked_mul(height)
      .ok_or_else(|| CodecError::Message("BCFNT texture sheet is impossibly large".to_owned()))?;

    Ok(Self {
      width,
      height,
      pixels: vec![[0, 0, 0, 0]; len],
    })
  }

  fn get_mut(&mut self, x: usize, y: usize) -> Option<&mut [u8; 4]> {
    if x >= self.width || y >= self.height {
      return None;
    }

    self.pixels.get_mut(y * self.width + x)
  }
}

/// Decodes one raw sheet into linear RGBA8 pixels.
pub(crate) fn decode_sheet(
  data: &[u8],
  width: u16,
  height: u16,
  format: TextureFormat,
  order: ByteOrder,
) -> Result<Sheet, CodecError> {
  if format.is_etc() {
    decode_etc(data, width, height, format, order)
  } else {
    decode_linear(data, width, height, format)
  }
}

/// The number of bytes one native texture sheet occupies.
///
/// Linear formats are swizzled over a power-of-two rounded sheet. ETC formats
/// store four 4x4 blocks in each 8x8 tile, with an extra 8-byte alpha block for
/// ETC1A4.
pub(crate) fn sheet_size(
  format: TextureFormat,
  width: u16,
  height: u16,
) -> Result<usize, CodecError> {
  if width == 0 || height == 0 {
    return Err(CodecError::Message(
      "BCFNT texture sheet has a zero dimension".to_owned(),
    ));
  }

  if format.is_etc() {
    let block_size = if format == TextureFormat::Etc1A4 {
      16
    } else {
      8
    };

    return tile_count(usize::from(width))?
      .checked_mul(tile_count(usize::from(height))?)
      .and_then(|count| count.checked_mul(4))
      .and_then(|count| count.checked_mul(block_size))
      .ok_or_else(|| CodecError::Message("BCFNT ETC1 sheet is impossibly large".to_owned()));
  }

  linear_payload_size(format, usize::from(width), usize::from(height))
}

/// Encodes linear RGBA8 pixels into the bytes of one sheet.
///
/// The returned bytes are exactly the native sheet payload. ETC1 and ETC1A4
/// recompress the pixels and are therefore lossy.
pub(crate) fn encode_sheet(
  sheet: &Sheet,
  format: TextureFormat,
  order: ByteOrder,
) -> Result<Vec<u8>, CodecError> {
  if format == TextureFormat::Hilo8 {
    return Err(CodecError::Message(
      "HILO8 texture sheets cannot be rebuilt from an image".to_owned(),
    ));
  }

  if !(8..=u16::MAX as usize).contains(&sheet.width)
    || !(8..=u16::MAX as usize).contains(&sheet.height)
  {
    return Err(CodecError::Message(
      "BCFNT texture sheets must be at least 8x8 pixels".to_owned(),
    ));
  }

  let expected = sheet_size(
    format,
    u16::try_from(sheet.width)
      .map_err(|_| CodecError::Message("BCFNT texture sheet is too wide".to_owned()))?,
    u16::try_from(sheet.height)
      .map_err(|_| CodecError::Message("BCFNT texture sheet is too tall".to_owned()))?,
  )?;

  if format.is_etc() {
    return encode_etc(sheet, format, order, expected);
  }

  let mut data = vec![0u8; expected];
  let rounded_width = rounded(sheet.width)?;
  let rounded_height = rounded(sheet.height)?;

  for tile_y in 0..rounded_height / 8 {
    for tile_x in 0..rounded_width / 8 {
      for block_y in 0..2 {
        for block_x in 0..2 {
          for group_y in 0..2 {
            for group_x in 0..2 {
              for pixel_y in 0..2 {
                for pixel_x in 0..2 {
                  let x = pixel_x + group_x * 2 + block_x * 4 + tile_x * 8;
                  let y = pixel_y + group_y * 2 + block_y * 4 + tile_y * 8;

                  if x >= sheet.width || y >= sheet.height {
                    continue;
                  }

                  let data_x = pixel_x + group_x * 4 + block_x * 16 + tile_x * 64;
                  let data_y =
                    pixel_y * 2 + group_y * 8 + block_y * 32 + tile_y * rounded_width * 8;
                  let pixel_index = data_x + data_y;

                  let pixel = *sheet.pixels.get(y * sheet.width + x).ok_or_else(|| {
                    CodecError::Message("BCFNT texture sheet pixel is out of bounds".to_owned())
                  })?;

                  write_pixel(&mut data, format, pixel_index, pixel)?;
                }
              }
            }
          }
        }
      }
    }
  }

  Ok(data)
}

pub(crate) fn sheet_to_png(sheet: &Sheet) -> Result<Vec<u8>, CodecError> {
  super::ensure_expanded_image_size(sheet.width as u16, sheet.height as u16, 1)?;
  let width = u32::try_from(sheet.width)
    .map_err(|_| CodecError::Message("BCFNT texture sheet is too wide for PNG".to_owned()))?;
  let height = u32::try_from(sheet.height)
    .map_err(|_| CodecError::Message("BCFNT texture sheet is too tall for PNG".to_owned()))?;

  let mut rgba = Vec::with_capacity(
    sheet
      .pixels
      .len()
      .checked_mul(4)
      .ok_or_else(|| CodecError::Message("BCFNT texture sheet is impossibly large".to_owned()))?,
  );

  for pixel in &sheet.pixels {
    rgba.extend_from_slice(pixel);
  }

  let mut out = Vec::new();
  {
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let mut writer = encoder
      .write_header()
      .map_err(|err| CodecError::Message(format!("Could not write a texture-sheet PNG: {err}")))?;

    writer
      .write_image_data(&rgba)
      .map_err(|err| CodecError::Message(format!("Could not write a texture-sheet PNG: {err}")))?;

    writer
      .finish()
      .map_err(|err| CodecError::Message(format!("Could not finish a texture-sheet PNG: {err}")))?;
  }

  Ok(out)
}

pub(crate) fn png_to_sheet(
  bytes: &[u8],
  expected_width: u16,
  expected_height: u16,
) -> Result<Sheet, CodecError> {
  let mut decoder = png::Decoder::new(Cursor::new(bytes));
  decoder.set_transformations(png::Transformations::normalize_to_color8());

  super::ensure_expanded_image_size(expected_width, expected_height, 1)?;
  let mut reader = decoder
    .read_info()
    .map_err(|err| CodecError::Message(format!("Could not read a texture-sheet PNG: {err}")))?;

  if reader.info().width != u32::from(expected_width)
    || reader.info().height != u32::from(expected_height)
  {
    return Err(CodecError::Message(format!(
      "Texture-sheet PNG is {}x{}, expected {expected_width}x{expected_height}",
      reader.info().width,
      reader.info().height
    )));
  }

  let buffer_size = reader
    .output_buffer_size()
    .ok_or_else(|| CodecError::Message("A texture-sheet PNG is too large to decode".to_owned()))?;
  let mut buffer = vec![0u8; buffer_size];

  let info = reader
    .next_frame(&mut buffer)
    .map_err(|err| CodecError::Message(format!("Could not decode a texture-sheet PNG: {err}")))?;

  if info.width != u32::from(expected_width) || info.height != u32::from(expected_height) {
    return Err(CodecError::Message(format!(
      "Texture-sheet PNG is {}x{}, expected {expected_width}x{expected_height}",
      info.width, info.height
    )));
  }

  if info.bit_depth != png::BitDepth::Eight {
    return Err(CodecError::Message(
      "Texture-sheet PNGs must be 8 bits per sample".to_owned(),
    ));
  }

  let mut sheet = Sheet::new(expected_width, expected_height)?;
  let payload = buffer
    .get(..info.buffer_size())
    .ok_or_else(|| CodecError::Message("Texture-sheet PNG data is truncated".to_owned()))?;

  for y in 0..sheet.height {
    let row_start = y
      .checked_mul(info.line_size)
      .ok_or_else(|| CodecError::Message("Texture-sheet PNG is impossibly large".to_owned()))?;
    let row = payload
      .get(row_start..)
      .and_then(|rest| rest.get(..info.line_size))
      .ok_or_else(|| CodecError::Message("Texture-sheet PNG data is truncated".to_owned()))?;

    for x in 0..sheet.width {
      let pixel = match info.color_type {
        png::ColorType::Rgba => {
          let offset = x.checked_mul(4).ok_or_else(|| {
            CodecError::Message("Texture-sheet PNG row is impossibly wide".to_owned())
          })?;
          let bytes = row
            .get(offset..offset + 4)
            .ok_or_else(|| CodecError::Message("Texture-sheet PNG row is truncated".to_owned()))?;
          [
            bytes[0],
            bytes[1],
            bytes[2],
            bytes[3],
          ]
        }
        png::ColorType::Rgb => {
          let offset = x.checked_mul(3).ok_or_else(|| {
            CodecError::Message("Texture-sheet PNG row is impossibly wide".to_owned())
          })?;
          let bytes = row
            .get(offset..offset + 3)
            .ok_or_else(|| CodecError::Message("Texture-sheet PNG row is truncated".to_owned()))?;
          [
            bytes[0],
            bytes[1],
            bytes[2],
            0xFF,
          ]
        }
        png::ColorType::GrayscaleAlpha => {
          let offset = x.checked_mul(2).ok_or_else(|| {
            CodecError::Message("Texture-sheet PNG row is impossibly wide".to_owned())
          })?;
          let bytes = row
            .get(offset..offset + 2)
            .ok_or_else(|| CodecError::Message("Texture-sheet PNG row is truncated".to_owned()))?;
          [
            bytes[0],
            bytes[0],
            bytes[0],
            bytes[1],
          ]
        }
        png::ColorType::Grayscale => {
          let value = *row
            .get(x)
            .ok_or_else(|| CodecError::Message("Texture-sheet PNG row is truncated".to_owned()))?;
          [
            value,
            value,
            value,
            0xFF,
          ]
        }
        png::ColorType::Indexed => {
          return Err(CodecError::Message(
            "Indexed texture-sheet PNGs are not supported".to_owned(),
          ));
        }
      };

      if let Some(target) = sheet.get_mut(x, y) {
        *target = pixel;
      }
    }
  }

  Ok(sheet)
}

fn decode_linear(
  data: &[u8],
  width: u16,
  height: u16,
  format: TextureFormat,
) -> Result<Sheet, CodecError> {
  if width < 8 || height < 8 {
    return Err(CodecError::Message(format!(
      "BCFNT {format:?} texture sheets narrower than 8x8 are not supported"
    )));
  }

  let mut sheet = Sheet::new(width, height)?;
  let rounded_width = rounded(sheet.width)?;
  let rounded_height = rounded(sheet.height)?;
  let expected = linear_payload_size(format, sheet.width, sheet.height)?;

  if data.len() < expected {
    return Err(CodecError::Message(format!(
      "BCFNT texture sheet is {} bytes, expected at least {expected}",
      data.len()
    )));
  }

  for tile_y in 0..rounded_height / 8 {
    for tile_x in 0..rounded_width / 8 {
      for block_y in 0..2 {
        for block_x in 0..2 {
          for group_y in 0..2 {
            for group_x in 0..2 {
              for pixel_y in 0..2 {
                for pixel_x in 0..2 {
                  let x = pixel_x + group_x * 2 + block_x * 4 + tile_x * 8;
                  let y = pixel_y + group_y * 2 + block_y * 4 + tile_y * 8;

                  if x >= sheet.width || y >= sheet.height {
                    continue;
                  }

                  let data_x = pixel_x + group_x * 4 + block_x * 16 + tile_x * 64;
                  let data_y =
                    pixel_y * 2 + group_y * 8 + block_y * 32 + tile_y * rounded_width * 8;
                  let pixel_index = data_x + data_y;

                  let pixel = read_pixel(data, format, pixel_index)?;

                  if let Some(target) = sheet.get_mut(x, y) {
                    *target = pixel;
                  }
                }
              }
            }
          }
        }
      }
    }
  }

  Ok(sheet)
}

/// Recompresses an RGBA8 sheet into native ETC1 or ETC1A4 blocks.
///
/// Each block is encoded by evaluating both ETC1 color layouts (individual and
/// differential) and both orientations, then keeping the candidate with the
/// lowest squared RGB error. The search is deliberately small and fully
/// deterministic; it is not a rate-distortion optimizer.
fn encode_etc(
  sheet: &Sheet,
  format: TextureFormat,
  order: ByteOrder,
  expected: usize,
) -> Result<Vec<u8>, CodecError> {
  let with_alpha = format == TextureFormat::Etc1A4;
  let tiles_x = tile_count(sheet.width)?;
  let tiles_y = tile_count(sheet.height)?;
  let mut out = Vec::with_capacity(expected);

  for tile_y in 0..tiles_y {
    for tile_x in 0..tiles_x {
      for block_y in 0..2 {
        for block_x in 0..2 {
          let mut block = [[0u8; 4]; 16];

          for pixel_y in 0..4usize {
            for pixel_x in 0..4usize {
              let x = pixel_x + block_x * 4 + tile_x * 8;
              let y = pixel_y + block_y * 4 + tile_y * 8;

              if x >= sheet.width || y >= sheet.height {
                continue;
              }

              let pixel = *sheet.pixels.get(y * sheet.width + x).ok_or_else(|| {
                CodecError::Message("BCFNT texture sheet pixel is out of bounds".to_owned())
              })?;
              block[pixel_x * 4 + pixel_y] = pixel;
            }
          }

          if with_alpha {
            let mut alphas = 0u64;
            for (offset, pixel) in block.iter().enumerate() {
              let alpha = (u64::from(pixel[3]) * 15 + 127) / 255;
              alphas |= alpha << (offset * 4);
            }
            write_u64(&mut out, alphas, order);
          }

          write_u64(&mut out, encode_etc_color(&block), order);
        }
      }
    }
  }

  if out.len() != expected {
    return Err(CodecError::Message(format!(
      "BCFNT ETC1 encoder produced {} bytes, expected {expected}",
      out.len()
    )));
  }

  Ok(out)
}

fn encode_etc_color(block: &[[u8; 4]; 16]) -> u64 {
  let mut best_bits = 0u64;
  let mut best_error = u64::MAX;

  for horizontal in [false, true] {
    for differential in [false, true] {
      let (bits, error) = encode_etc_candidate(block, horizontal, differential);
      if error < best_error {
        best_error = error;
        best_bits = bits;
      }
    }
  }

  best_bits
}

fn encode_etc_candidate(block: &[[u8; 4]; 16], horizontal: bool, differential: bool) -> (u64, u64) {
  let first = etc_region(horizontal, true);
  let second = etc_region(horizontal, false);
  let mean_first = etc_region_mean(block, &first);
  let mean_second = etc_region_mean(block, &second);

  let (base_first, base_second, colors) = if differential {
    let first_codes = [
      quantize(mean_first[0], 5),
      quantize(mean_first[1], 5),
      quantize(mean_first[2], 5),
    ];
    let second_target = [
      quantize(mean_second[0], 5),
      quantize(mean_second[1], 5),
      quantize(mean_second[2], 5),
    ];
    let deltas = [
      (second_target[0] - first_codes[0]).clamp(-4, 3),
      (second_target[1] - first_codes[1]).clamp(-4, 3),
      (second_target[2] - first_codes[2]).clamp(-4, 3),
    ];
    let second_codes = [
      first_codes[0] + deltas[0],
      first_codes[1] + deltas[1],
      first_codes[2] + deltas[2],
    ];

    (
      [
        expand_five_code(first_codes[0]),
        expand_five_code(first_codes[1]),
        expand_five_code(first_codes[2]),
      ],
      [
        expand_five_code(second_codes[0]),
        expand_five_code(second_codes[1]),
        expand_five_code(second_codes[2]),
      ],
      EtcColors::Differential {
        first_codes,
        deltas,
      },
    )
  } else {
    let first_codes = [
      quantize(mean_first[0], 4),
      quantize(mean_first[1], 4),
      quantize(mean_first[2], 4),
    ];
    let second_codes = [
      quantize(mean_second[0], 4),
      quantize(mean_second[1], 4),
      quantize(mean_second[2], 4),
    ];

    (
      [
        first_codes[0] * 17,
        first_codes[1] * 17,
        first_codes[2] * 17,
      ],
      [
        second_codes[0] * 17,
        second_codes[1] * 17,
        second_codes[2] * 17,
      ],
      EtcColors::Individual {
        first_codes,
        second_codes,
      },
    )
  };

  let (first_error, first_table, first_amounts, first_signs) =
    encode_etc_region(block, &first, base_first);
  let (second_error, second_table, second_amounts, second_signs) =
    encode_etc_region(block, &second, base_second);

  let amounts = first_amounts | second_amounts;
  let signs = first_signs | second_signs;
  let mut bits = u64::from(amounts)
    | (u64::from(signs) << 16)
    | ((first_table as u64) << ETC_TABLE1_OFFSET)
    | ((second_table as u64) << ETC_TABLE2_OFFSET);

  if horizontal {
    bits |= 1 << ETC_ORIENTATION_BIT;
  }
  if differential {
    bits |= 1 << ETC_DIFFERENTIAL_BIT;
  }

  match colors {
    EtcColors::Differential {
      first_codes,
      deltas,
    } => {
      bits |= (u64::from((deltas[2] & 0x07) as u8) << ETC_BLUE_OFFSET)
        | (u64::from((deltas[1] & 0x07) as u8) << ETC_GREEN2_OFFSET)
        | (u64::from((deltas[0] & 0x07) as u8) << ETC_RED2_OFFSET)
        | (u64::from(first_codes[2] as u8) << ETC_DIFF_BLUE_OFFSET)
        | (u64::from(first_codes[1] as u8) << ETC_DIFF_GREEN1_OFFSET)
        | (u64::from(first_codes[0] as u8) << ETC_DIFF_RED1_OFFSET);
    }
    EtcColors::Individual {
      first_codes,
      second_codes,
    } => {
      bits |= (u64::from(second_codes[2] as u8) << ETC_BLUE_OFFSET)
        | (u64::from(first_codes[2] as u8) << ETC_INDIV_BLUE1_OFFSET)
        | (u64::from(second_codes[1] as u8) << ETC_GREEN2_OFFSET)
        | (u64::from(first_codes[1] as u8) << ETC_INDIV_GREEN1_OFFSET)
        | (u64::from(second_codes[0] as u8) << ETC_RED2_OFFSET)
        | (u64::from(first_codes[0] as u8) << ETC_INDIV_RED1_OFFSET);
    }
  }

  (bits, first_error + second_error)
}

enum EtcColors {
  Differential {
    first_codes: [i32; 3],
    deltas: [i32; 3],
  },
  Individual {
    first_codes: [i32; 3],
    second_codes: [i32; 3],
  },
}

fn etc_region(horizontal: bool, first: bool) -> [usize; 8] {
  let mut offsets = [0usize; 8];
  let mut next = 0usize;

  for pixel_y in 0..4usize {
    for pixel_x in 0..4usize {
      let belongs = if horizontal { pixel_y < 2 } else { pixel_x < 2 };

      if belongs == first {
        if let Some(offset) = offsets.get_mut(next) {
          *offset = pixel_x * 4 + pixel_y;
        }
        next += 1;
      }
    }
  }

  offsets
}

fn etc_region_mean(block: &[[u8; 4]; 16], offsets: &[usize; 8]) -> [i32; 3] {
  let mut sums = [0u32; 3];

  for offset in offsets {
    if let Some(pixel) = block.get(*offset) {
      for channel in 0..3 {
        sums[channel] += u32::from(pixel[channel]);
      }
    }
  }

  [
    ((sums[0] + 4) / 8) as i32,
    ((sums[1] + 4) / 8) as i32,
    ((sums[2] + 4) / 8) as i32,
  ]
}

fn encode_etc_region(
  block: &[[u8; 4]; 16],
  offsets: &[usize; 8],
  base: [i32; 3],
) -> (u64, usize, u16, u16) {
  let mut best_error = u64::MAX;
  let mut best_table = 0usize;
  let mut best_amounts = 0u16;
  let mut best_signs = 0u16;

  for (table, modifiers) in ETC_MODIFIERS.iter().enumerate() {
    let mut error = 0u64;
    let mut amounts = 0u16;
    let mut signs = 0u16;

    for offset in offsets {
      let Some(pixel) = block.get(*offset) else {
        continue;
      };

      let mut best_pixel_error = u64::MAX;
      let mut best_large = false;
      let mut best_negative = false;

      for (large, modifier) in [false, true].into_iter().zip(modifiers) {
        for (negative, signed) in [false, true].into_iter().zip([
          *modifier,
          -*modifier,
        ]) {
          let mut pixel_error = 0u64;

          for channel in 0..3 {
            let decoded = clamp(base[channel] + signed);
            let difference = i64::from(decoded) - i64::from(pixel[channel]);
            pixel_error += (difference * difference) as u64;
          }

          if pixel_error < best_pixel_error {
            best_pixel_error = pixel_error;
            best_large = large;
            best_negative = negative;
          }
        }
      }

      error += best_pixel_error;
      if best_large {
        amounts |= 1 << *offset;
      }
      if best_negative {
        signs |= 1 << *offset;
      }
    }

    if error < best_error {
      best_error = error;
      best_table = table;
      best_amounts = amounts;
      best_signs = signs;
    }
  }

  (best_error, best_table, best_amounts, best_signs)
}

fn quantize(value: i32, bits: u32) -> i32 {
  let maximum = (1 << bits) - 1;
  ((value * maximum + 127) / 255).clamp(0, maximum)
}

fn expand_five_code(value: i32) -> i32 {
  (value << 3) | (value >> 2)
}

fn decode_etc(
  data: &[u8],
  width: u16,
  height: u16,
  format: TextureFormat,
  order: ByteOrder,
) -> Result<Sheet, CodecError> {
  let mut sheet = Sheet::new(width, height)?;
  let with_alpha = format == TextureFormat::Etc1A4;
  let block_size = if with_alpha { 16 } else { 8 };

  let tiles_x = tile_count(sheet.width)?;
  let tiles_y = tile_count(sheet.height)?;
  let expected = tiles_x
    .checked_mul(tiles_y)
    .and_then(|count| count.checked_mul(4))
    .and_then(|count| count.checked_mul(block_size))
    .ok_or_else(|| CodecError::Message("BCFNT ETC1 sheet is impossibly large".to_owned()))?;

  if data.len() < expected {
    return Err(CodecError::Message(format!(
      "BCFNT ETC1 texture sheet is {} bytes, expected at least {expected}",
      data.len()
    )));
  }

  let mut position = 0usize;

  for tile_y in 0..tiles_y {
    for tile_x in 0..tiles_x {
      for block_y in 0..2 {
        for block_x in 0..2 {
          let block = data
            .get(position..position + block_size)
            .ok_or_else(|| CodecError::Message("BCFNT ETC1 sheet is truncated".to_owned()))?;
          position += block_size;

          let (alphas, color_bytes) = if with_alpha {
            let (alpha, color) = block.split_at(8);
            (read_u64(alpha, order)?, color)
          } else {
            (u64::MAX, block)
          };

          let pixels = read_u64(color_bytes, order)?;

          let differential = (pixels >> ETC_DIFFERENTIAL_BIT) & 0x01 == 1;
          let horizontal = (pixels >> ETC_ORIENTATION_BIT) & 0x01 == 1;
          let table1 = ETC_MODIFIERS[((pixels >> ETC_TABLE1_OFFSET) & 0x07) as usize];
          let table2 = ETC_MODIFIERS[((pixels >> ETC_TABLE2_OFFSET) & 0x07) as usize];

          let (color1, color2) = if differential {
            let mut red = ((pixels >> ETC_DIFF_RED1_OFFSET) & 0x1F) as i32;
            let mut green = ((pixels >> ETC_DIFF_GREEN1_OFFSET) & 0x1F) as i32;
            let mut blue = ((pixels >> ETC_DIFF_BLUE_OFFSET) & 0x1F) as i32;

            let color1 = [
              expand_five_signed(red),
              expand_five_signed(green),
              expand_five_signed(blue),
            ];

            red += complement(((pixels >> ETC_RED2_OFFSET) & 0x07) as i32, 3);
            green += complement(((pixels >> ETC_GREEN2_OFFSET) & 0x07) as i32, 3);
            blue += complement(((pixels >> ETC_BLUE_OFFSET) & 0x07) as i32, 3);

            (
              color1,
              [
                expand_five_signed(red),
                expand_five_signed(green),
                expand_five_signed(blue),
              ],
            )
          } else {
            (
              [
                ((pixels >> ETC_INDIV_RED1_OFFSET) & 0x0F) as i32 * 0x11,
                ((pixels >> ETC_INDIV_GREEN1_OFFSET) & 0x0F) as i32 * 0x11,
                ((pixels >> ETC_INDIV_BLUE1_OFFSET) & 0x0F) as i32 * 0x11,
              ],
              [
                ((pixels >> ETC_RED2_OFFSET) & 0x0F) as i32 * 0x11,
                ((pixels >> ETC_GREEN2_OFFSET) & 0x0F) as i32 * 0x11,
                ((pixels >> ETC_BLUE_OFFSET) & 0x0F) as i32 * 0x11,
              ],
            )
          };

          let amounts = (pixels & 0xFFFF) as u32;
          let signs = ((pixels >> 16) & 0xFFFF) as u32;

          for pixel_y in 0..4usize {
            for pixel_x in 0..4usize {
              let x = pixel_x + block_x * 4 + tile_x * 8;
              let y = pixel_y + block_y * 4 + tile_y * 8;

              if x >= sheet.width || y >= sheet.height {
                continue;
              }

              let offset = (pixel_x * 4 + pixel_y) as u32;

              let (table, color) = if horizontal {
                if pixel_y < 2 {
                  (table1, color1)
                } else {
                  (table2, color2)
                }
              } else if pixel_x < 2 {
                (table1, color1)
              } else {
                (table2, color2)
              };

              let mut amount = table[((amounts >> offset) & 0x01) as usize];
              if (signs >> offset) & 0x01 == 1 {
                amount = -amount;
              }

              let red = clamp(color[0] + amount);
              let green = clamp(color[1] + amount);
              let blue = clamp(color[2] + amount);
              let alpha = (((alphas >> (offset * 4)) & 0x0F) * 0x11) as u8;

              if let Some(target) = sheet.get_mut(x, y) {
                *target = [
                  red,
                  green,
                  blue,
                  alpha,
                ];
              }
            }
          }
        }
      }
    }
  }

  Ok(sheet)
}

fn read_pixel(
  data: &[u8],
  format: TextureFormat,
  pixel_index: usize,
) -> Result<[u8; 4], CodecError> {
  let truncated = || CodecError::Message("BCFNT texture sheet is truncated".to_owned());

  Ok(match format {
    TextureFormat::Rgba8 => {
      let offset = pixel_index.checked_mul(4).ok_or_else(truncated)?;
      let bytes = data.get(offset..offset + 4).ok_or_else(truncated)?;
      [
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
      ]
    }
    TextureFormat::Rgb8 => {
      let offset = pixel_index.checked_mul(3).ok_or_else(truncated)?;
      let bytes = data.get(offset..offset + 3).ok_or_else(truncated)?;
      [
        bytes[0],
        bytes[1],
        bytes[2],
        0xFF,
      ]
    }
    TextureFormat::Rgba5551 => {
      let offset = pixel_index.checked_mul(2).ok_or_else(truncated)?;
      let bytes = data.get(offset..offset + 2).ok_or_else(truncated)?;
      let first = bytes[0];
      let second = bytes[1];

      [
        expand_five(i64::from((first >> 3) & 0x1F)),
        expand_five(i64::from(((first & 0x07) << 2) | ((second >> 6) & 0x03))),
        expand_five(i64::from((second >> 1) & 0x1F)),
        (second & 0x01) * 0xFF,
      ]
    }
    TextureFormat::Rgb565 => {
      let offset = pixel_index.checked_mul(2).ok_or_else(truncated)?;
      let bytes = data.get(offset..offset + 2).ok_or_else(truncated)?;
      let first = bytes[0];
      let second = bytes[1];

      [
        expand_five(i64::from((first >> 3) & 0x1F)),
        expand_six(i64::from(((first & 0x07) << 3) | ((second >> 5) & 0x07))),
        expand_five(i64::from(second & 0x1F)),
        0xFF,
      ]
    }
    TextureFormat::Rgba4 => {
      let offset = pixel_index.checked_mul(2).ok_or_else(truncated)?;
      let bytes = data.get(offset..offset + 2).ok_or_else(truncated)?;
      let first = bytes[0];
      let second = bytes[1];

      [
        ((first >> 4) & 0x0F) * 0x11,
        (first & 0x0F) * 0x11,
        ((second >> 4) & 0x0F) * 0x11,
        (second & 0x0F) * 0x11,
      ]
    }
    TextureFormat::La8 => {
      let offset = pixel_index.checked_mul(2).ok_or_else(truncated)?;
      let bytes = data.get(offset..offset + 2).ok_or_else(truncated)?;
      [
        bytes[0],
        bytes[0],
        bytes[0],
        bytes[1],
      ]
    }
    TextureFormat::Hilo8 => {
      return Err(CodecError::Message(
        "HILO8 texture sheets are not supported".to_owned(),
      ));
    }
    TextureFormat::L8 => {
      let value = *data.get(pixel_index).ok_or_else(truncated)?;
      [
        value,
        value,
        value,
        0xFF,
      ]
    }
    TextureFormat::A8 => {
      let value = *data.get(pixel_index).ok_or_else(truncated)?;
      [
        0xFF,
        0xFF,
        0xFF,
        value,
      ]
    }
    TextureFormat::La4 => {
      let value = *data.get(pixel_index).ok_or_else(truncated)?;
      [
        ((value >> 4) & 0x0F) * 0x11,
        ((value >> 4) & 0x0F) * 0x11,
        ((value >> 4) & 0x0F) * 0x11,
        (value & 0x0F) * 0x11,
      ]
    }
    TextureFormat::L4 => {
      let value = *data.get(pixel_index / 2).ok_or_else(truncated)?;
      let shift = (pixel_index & 1) * 4;
      let luminance = ((value >> shift) & 0x0F) * 0x11;
      [
        luminance,
        luminance,
        luminance,
        0xFF,
      ]
    }
    TextureFormat::A4 => {
      let value = *data.get(pixel_index / 2).ok_or_else(truncated)?;
      let shift = (pixel_index & 1) * 4;
      let alpha = ((value >> shift) & 0x0F) * 0x11;
      [
        0xFF,
        0xFF,
        0xFF,
        alpha,
      ]
    }
    TextureFormat::Etc1 | TextureFormat::Etc1A4 => {
      return Err(CodecError::Message(
        "ETC1 texture sheets are decoded separately".to_owned(),
      ));
    }
  })
}

fn write_pixel(
  data: &mut [u8],
  format: TextureFormat,
  pixel_index: usize,
  pixel: [u8; 4],
) -> Result<(), CodecError> {
  let truncated = || CodecError::Message("BCFNT texture sheet is too small".to_owned());
  let [
    red,
    green,
    blue,
    alpha,
  ] = pixel;

  match format {
    TextureFormat::Rgba8 => {
      let offset = pixel_index.checked_mul(4).ok_or_else(truncated)?;
      let target = data.get_mut(offset..offset + 4).ok_or_else(truncated)?;
      target.copy_from_slice(&pixel);
    }
    TextureFormat::Rgb8 => {
      let offset = pixel_index.checked_mul(3).ok_or_else(truncated)?;
      let target = data.get_mut(offset..offset + 3).ok_or_else(truncated)?;
      target.copy_from_slice(&[red, green, blue]);
    }
    TextureFormat::Rgba5551 => {
      let offset = pixel_index.checked_mul(2).ok_or_else(truncated)?;
      let target = data.get_mut(offset..offset + 2).ok_or_else(truncated)?;
      let red5 = red >> 3;
      let green5 = green >> 3;
      let blue5 = blue >> 3;
      let alpha1 = u8::from(alpha > 0);

      target[0] = (red5 << 3) | (green5 >> 2);
      target[1] = (green5 << 6) | (blue5 << 1) | alpha1;
    }
    TextureFormat::Rgb565 => {
      let offset = pixel_index.checked_mul(2).ok_or_else(truncated)?;
      let target = data.get_mut(offset..offset + 2).ok_or_else(truncated)?;
      let red5 = red >> 3;
      let green6 = green >> 2;
      let blue5 = blue >> 3;

      target[0] = (red5 << 3) | (green6 >> 3);
      target[1] = (green6 << 5) | blue5;
    }
    TextureFormat::Rgba4 => {
      let offset = pixel_index.checked_mul(2).ok_or_else(truncated)?;
      let target = data.get_mut(offset..offset + 2).ok_or_else(truncated)?;
      let red4 = red >> 4;
      let green4 = green >> 4;
      let blue4 = blue >> 4;
      let alpha4 = alpha >> 4;

      target[0] = (red4 << 4) | green4;
      target[1] = (blue4 << 4) | alpha4;
    }
    TextureFormat::La8 => {
      let offset = pixel_index.checked_mul(2).ok_or_else(truncated)?;
      let target = data.get_mut(offset..offset + 2).ok_or_else(truncated)?;
      target.copy_from_slice(&[
        luminance(pixel),
        alpha,
      ]);
    }
    TextureFormat::Hilo8 => {
      return Err(CodecError::Message(
        "HILO8 texture sheets cannot be rebuilt from an image".to_owned(),
      ));
    }
    TextureFormat::L8 => {
      let target = data.get_mut(pixel_index).ok_or_else(truncated)?;
      *target = luminance(pixel);
    }
    TextureFormat::A8 => {
      let target = data.get_mut(pixel_index).ok_or_else(truncated)?;
      *target = alpha;
    }
    TextureFormat::La4 => {
      let target = data.get_mut(pixel_index).ok_or_else(truncated)?;
      let light = (luminance(pixel) >> 4) & 0x0F;
      let alpha = (alpha >> 4) & 0x0F;
      *target = (light << 4) | alpha;
    }
    TextureFormat::L4 => {
      let target = data.get_mut(pixel_index / 2).ok_or_else(truncated)?;
      let shift = (pixel_index & 1) * 4;
      let light = (luminance(pixel) >> 4) & 0x0F;
      *target |= light << shift;
    }
    TextureFormat::A4 => {
      let target = data.get_mut(pixel_index / 2).ok_or_else(truncated)?;
      let shift = (pixel_index & 1) * 4;
      let alpha = (alpha >> 4) & 0x0F;
      *target |= alpha << shift;
    }
    TextureFormat::Etc1 | TextureFormat::Etc1A4 => {
      return Err(CodecError::Message(
        "ETC1 texture sheets are encoded as 4x4 blocks".to_owned(),
      ));
    }
  }

  Ok(())
}

fn linear_payload_size(
  format: TextureFormat,
  width: usize,
  height: usize,
) -> Result<usize, CodecError> {
  let bits = format
    .bits_per_pixel()
    .ok_or_else(|| CodecError::Message(format!("{format:?} texture sheets are not linear")))?;

  let rounded_width = rounded(width)?;
  let rounded_height = rounded(height)?;

  rounded_width
    .checked_mul(rounded_height)
    .and_then(|pixels| pixels.checked_mul(bits))
    .and_then(|bits| bits.checked_add(7))
    .map(|bits| bits / 8)
    .ok_or_else(|| CodecError::Message("BCFNT texture sheet is impossibly large".to_owned()))
}

fn rounded(value: usize) -> Result<usize, CodecError> {
  if value == 0 {
    return Err(CodecError::Message(
      "BCFNT texture sheet has a zero dimension".to_owned(),
    ));
  }

  value
    .checked_next_power_of_two()
    .ok_or_else(|| CodecError::Message("BCFNT texture sheet is impossibly large".to_owned()))
}

fn tile_count(value: usize) -> Result<usize, CodecError> {
  if value == 0 {
    return Err(CodecError::Message(
      "BCFNT texture sheet has a zero dimension".to_owned(),
    ));
  }

  value
    .checked_add(7)
    .map(|value| value / 8)
    .and_then(usize::checked_next_power_of_two)
    .ok_or_else(|| CodecError::Message("BCFNT ETC1 sheet is impossibly large".to_owned()))
}

/// Perceptual luminance, rounded to the nearest integer.
///
/// The coefficients sum to exactly 10,000, so a gray pixel maps back to its own
/// value. Doing this in floating point and truncating does not: the sum can land
/// just below the input (255 becomes 254.999...), which silently loses one LSB
/// when a decoded LA8 or L8 sheet is re-encoded.
fn luminance(pixel: [u8; 4]) -> u8 {
  let [red, green, blue, _] = pixel;
  let weighted = 2126 * u32::from(red) + 7152 * u32::from(green) + 722 * u32::from(blue);

  ((weighted + 5000) / 10000) as u8
}

fn expand_five_signed(value: i32) -> i32 {
  (value << 3) | ((value >> 2) & 0x07)
}

fn expand_five(value: i64) -> u8 {
  if value < 0 {
    return 0;
  }

  let value = value.min(0x1F) as u8;
  (value << 3) | (value >> 2)
}

fn expand_six(value: i64) -> u8 {
  if value < 0 {
    return 0;
  }

  let value = value.min(0x3F) as u8;
  (value << 2) | (value >> 4)
}

fn clamp(value: i32) -> u8 {
  value.clamp(0, 0xFF) as u8
}

fn complement(value: i32, bits: u32) -> i32 {
  if value >> (bits - 1) == 0 {
    value
  } else {
    value - (1 << bits)
  }
}

fn read_u64(data: &[u8], order: ByteOrder) -> Result<u64, CodecError> {
  let bytes: [u8; 8] = data
    .get(..8)
    .and_then(|bytes| bytes.try_into().ok())
    .ok_or_else(|| CodecError::Message("BCFNT ETC1 block is truncated".to_owned()))?;

  Ok(match order {
    ByteOrder::Little => u64::from_le_bytes(bytes),
    ByteOrder::Big => u64::from_be_bytes(bytes),
  })
}

fn write_u64(out: &mut Vec<u8>, value: u64, order: ByteOrder) {
  let bytes = match order {
    ByteOrder::Little => value.to_le_bytes(),
    ByteOrder::Big => value.to_be_bytes(),
  };
  out.extend_from_slice(&bytes);
}
