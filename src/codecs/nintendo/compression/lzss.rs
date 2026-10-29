//! Nintendo LZ10/LZ11 LZSS engine.
//!
//! Both formats share a byte-oriented token stream behind a four-byte header:
//! the format byte (`0x10` or `0x11`), then the decompressed size as a 24-bit
//! little-endian value. Each flag byte introduces up to eight tokens, most
//! significant bit first. A zero bit is a literal byte; a one bit is a back
//! reference that copies from the already produced output.
//!
//! LZ10 stores a reference as one big-endian 16-bit word: a four-bit
//! `length - 3` and a twelve-bit `displacement - 1`, so a match is 3..=18 bytes
//! long and reaches at most 4096 bytes back. LZ11 uses three reference shapes
//! selected by the high nibble of its first byte: `0x0` for a 24-bit form
//! (17..=272 bytes), `0x1` for a 32-bit form (273..=65808 bytes), and anything
//! else for the LZ10-style short form (3..=16 bytes). All three keep a
//! twelve-bit `displacement - 1`.
//!
//! The decompressor is bounds-checked and refuses a reference that reaches
//! before the start of the output or a token that would overrun the declared
//! size. The compressor is a greedy hash-chain matcher; it produces a valid
//! stream but not necessarily the same bytes as Nintendo's compressor.
//!
//! Attribution: the LZ10/LZ11 layouts and the decompression rules are ported
//! from magical's nlzss (`lzss.py`, `compress.py`, MIT licence); see
//! ATTRIBUTIONS.md. The matcher here is an independent hash-chain
//! implementation.

use crate::codecs::errors::CodecError;

#[cfg(test)]
mod tests;

const HEADER_SIZE: usize = 4;
const WINDOW_SIZE: usize = 4096;
const MIN_MATCH: usize = 3;
const HASH_BITS: usize = 16;
const HASH_SIZE: usize = 1 << HASH_BITS;
const CHAIN_LIMIT: usize = 128;
const NONE: u32 = u32::MAX;
const MAX_DECOMPRESSED_SIZE: usize = 0xFF_FFFF;

/// The two LZSS variants, which differ only in their reference encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
  Lz10,
  Lz11,
}

impl Format {
  pub(crate) fn magic(self) -> u8 {
    match self {
      Self::Lz10 => 0x10,
      Self::Lz11 => 0x11,
    }
  }

  pub(crate) fn label(self) -> &'static str {
    match self {
      Self::Lz10 => "LZ10",
      Self::Lz11 => "LZ11",
    }
  }

  /// The longest match the reference encoding can express, inclusive.
  fn max_match(self) -> usize {
    match self {
      // length - 3 fits in four bits.
      Self::Lz10 => 0x12,
      // 0x111 + 0xFFFF is the largest 16-bit stored length.
      Self::Lz11 => 0x1_0110,
    }
  }
}

struct Reader<'a> {
  data: &'a [u8],
  position: usize,
}

impl<'a> Reader<'a> {
  fn new(data: &'a [u8]) -> Self {
    Self { data, position: 0 }
  }

  fn byte(&mut self, format: Format) -> Result<u8, CodecError> {
    let byte = self.data.get(self.position).copied().ok_or_else(|| {
      CodecError::Message(format!(
        "{} stream ended in the middle of a token at offset {:#x}",
        format.label(),
        self.position + HEADER_SIZE
      ))
    })?;
    self.position += 1;
    Ok(byte)
  }

  fn word_be(&mut self, format: Format) -> Result<u16, CodecError> {
    let high = self.byte(format)?;
    let low = self.byte(format)?;
    Ok(u16::from_be_bytes([high, low]))
  }
}

/// Decompresses an LZ10 or LZ11 stream.
pub(crate) fn decompress(data: &[u8], format: Format) -> Result<Vec<u8>, CodecError> {
  let header = data.get(..HEADER_SIZE).ok_or_else(|| {
    CodecError::Message(format!(
      "{} stream is truncated: {} bytes (minimum: {HEADER_SIZE})",
      format.label(),
      data.len()
    ))
  })?;
  if header[0] != format.magic() {
    return Err(CodecError::Message(format!(
      "Not an {} stream: the first byte is {:#04x}, expected {:#04x}",
      format.label(),
      header[0],
      format.magic()
    )));
  }

  // The top byte of the little-endian size is occupied by the format byte.
  let size = u32::from_le_bytes([
    header[1],
    header[2],
    header[3],
    0,
  ]) as usize;
  let mut reader = Reader::new(&data[HEADER_SIZE..]);
  let mut output = Vec::with_capacity(size);

  while output.len() < size {
    let flags = reader.byte(format)?;
    for bit in 0..8 {
      if output.len() >= size {
        break;
      }
      if flags & (0x80 >> bit) == 0 {
        output.push(reader.byte(format)?);
      } else {
        let (count, displacement) = read_reference(&mut reader, format)?;
        copy_reference(&mut output, count, displacement, size, format)?;
      }
    }
  }

  Ok(output)
}

fn read_reference(reader: &mut Reader<'_>, format: Format) -> Result<(usize, usize), CodecError> {
  match format {
    Format::Lz10 => {
      let word = reader.word_be(format)?;
      let count = usize::from(word >> 12) + MIN_MATCH;
      let displacement = usize::from(word & 0x0FFF) + 1;
      Ok((count, displacement))
    }
    Format::Lz11 => {
      let first = reader.byte(format)?;
      let indicator = first >> 4;
      match indicator {
        0 => {
          let second = reader.byte(format)?;
          let count = (usize::from(first) << 4) + usize::from(second >> 4) + 0x11;
          let displacement =
            (usize::from(second & 0x0F) << 8) + usize::from(reader.byte(format)?) + 1;
          Ok((count, displacement))
        }
        1 => {
          let second = reader.byte(format)?;
          let third = reader.byte(format)?;
          let count = (usize::from(first & 0x0F) << 12)
            + (usize::from(second) << 4)
            + usize::from(third >> 4)
            + 0x111;
          let displacement =
            (usize::from(third & 0x0F) << 8) + usize::from(reader.byte(format)?) + 1;
          Ok((count, displacement))
        }
        _ => {
          let count = usize::from(indicator) + 1;
          let displacement =
            (usize::from(first & 0x0F) << 8) + usize::from(reader.byte(format)?) + 1;
          Ok((count, displacement))
        }
      }
    }
  }
}

fn copy_reference(
  output: &mut Vec<u8>,
  count: usize,
  displacement: usize,
  size: usize,
  format: Format,
) -> Result<(), CodecError> {
  if displacement == 0 || displacement > output.len() {
    return Err(CodecError::Message(format!(
      "{} back reference at output offset {:#x} reaches {} bytes back, but only {} bytes are available",
      format.label(),
      output.len(),
      displacement,
      output.len()
    )));
  }
  let end = output.len().checked_add(count).ok_or_else(|| {
    CodecError::Message(format!(
      "{} token at output offset {:#x} overflows the output size",
      format.label(),
      output.len()
    ))
  })?;
  if end > size {
    return Err(CodecError::Message(format!(
      "{} token at output offset {:#x} would produce {} bytes, past the declared size {size}",
      format.label(),
      output.len(),
      end
    )));
  }
  // A back reference may overlap the bytes it is producing; reading one byte
  // at a time from the growing output reproduces that behaviour.
  let start = output.len() - displacement;
  for index in 0..count {
    let byte = output[start + index];
    output.push(byte);
  }
  Ok(())
}

/// Compresses `data` into an LZ10 or LZ11 stream.
pub(crate) fn compress(data: &[u8], format: Format) -> Result<Vec<u8>, CodecError> {
  if data.len() > MAX_DECOMPRESSED_SIZE {
    return Err(CodecError::Message(format!(
      "{} cannot encode {} bytes: the header stores at most {MAX_DECOMPRESSED_SIZE}",
      format.label(),
      data.len()
    )));
  }

  let mut body = Vec::with_capacity(data.len());
  let mut head = vec![NONE; HASH_SIZE];
  let mut previous = vec![NONE; data.len()];
  let mut group = [Token::Literal(0); 8];
  let mut group_len = 0usize;
  let mut flags = 0u8;
  let mut position = 0usize;

  while position < data.len() {
    let reference = find_match(data, &head, &previous, position, format);
    let token = if let Some((count, displacement)) = reference {
      for index in 0..count {
        insert(data, &mut head, &mut previous, position + index)?;
      }
      position += count;
      Token::Reference(count, displacement)
    } else {
      let byte = data[position];
      insert(data, &mut head, &mut previous, position)?;
      position += 1;
      Token::Literal(byte)
    };

    flags = (flags << 1) | u8::from(matches!(token, Token::Reference(..)));
    group[group_len] = token;
    group_len += 1;
    if group_len == 8 {
      write_group(&mut body, flags, &group[..group_len], format)?;
      flags = 0;
      group_len = 0;
    }
  }

  if group_len > 0 {
    // Unused low bits of the flag byte are zero.
    flags <<= 8 - group_len;
    write_group(&mut body, flags, &group[..group_len], format)?;
  }

  // Nintendo's compressor pads the token stream to a four-byte boundary with
  // 0xFF. The padding is never read back, but keeping it matches convention.
  body.resize(body.len().next_multiple_of(4), 0xFF);

  // The header stores the decompressed size in 24 bits; the length check
  // above keeps it inside that field.
  let size = u32::try_from(data.len()).map_err(|_| {
    CodecError::Message(format!(
      "{} cannot encode {} bytes: the header stores at most {MAX_DECOMPRESSED_SIZE}",
      format.label(),
      data.len()
    ))
  })?;
  let mut output = Vec::with_capacity(HEADER_SIZE + body.len());
  output.push(format.magic());
  output.extend_from_slice(&size.to_le_bytes()[..3]);
  output.extend_from_slice(&body);
  Ok(output)
}

#[derive(Clone, Copy)]
enum Token {
  Literal(u8),
  Reference(usize, usize),
}

fn write_group(
  body: &mut Vec<u8>,
  flags: u8,
  tokens: &[Token],
  format: Format,
) -> Result<(), CodecError> {
  body.push(flags);
  for token in tokens {
    match *token {
      Token::Literal(byte) => body.push(byte),
      Token::Reference(count, displacement) => {
        write_reference(body, count, displacement, format)?;
      }
    }
  }
  Ok(())
}

/// Writes a back reference, whose fields are sized by the format.
///
/// The matcher only produces tokens the reference encoding can express, but a
/// token that does not fit is refused rather than packed wrapped.
fn write_reference(
  body: &mut Vec<u8>,
  count: usize,
  displacement: usize,
  format: Format,
) -> Result<(), CodecError> {
  debug_assert!((MIN_MATCH..=format.max_match()).contains(&count));
  debug_assert!((1..=WINDOW_SIZE).contains(&displacement));
  let out_of_range = |what: &str, value: usize| {
    CodecError::Message(format!(
      "{} match {what} {value} cannot be encoded",
      format.label()
    ))
  };
  // Every form stores `displacement - 1` in a twelve-bit field, which covers
  // the whole window.
  let displacement =
    u16::try_from(displacement - 1).map_err(|_| out_of_range("displacement", displacement))?;
  match format {
    // `length - 3` fits in four bits.
    Format::Lz10 => {
      let length = u16::try_from(count - MIN_MATCH).map_err(|_| out_of_range("length", count))?;
      body.extend_from_slice(&((length << 12) | displacement).to_be_bytes());
    }
    // `length - 1` fills the high nibble.
    Format::Lz11 if count <= 0x10 => {
      let length = u16::try_from(count - 1).map_err(|_| out_of_range("length", count))?;
      body.extend_from_slice(&((length << 12) | displacement).to_be_bytes());
    }
    // `length - 0x11` takes eight bits, in front of the displacement; the top
    // nibble of the packed word stays zero, which is the 0x0 indicator.
    Format::Lz11 if count <= 0x110 => {
      let length = u16::try_from(count - 0x11).map_err(|_| out_of_range("length", count))?;
      let word = (u32::from(length) << 12) | u32::from(displacement);
      body.extend_from_slice(&word.to_be_bytes()[1..]);
    }
    // `length - 0x111` takes sixteen bits, behind the 0x1 indicator.
    Format::Lz11 => {
      let length = u16::try_from(count - 0x111).map_err(|_| out_of_range("length", count))?;
      let word = 0x1000_0000 | (u32::from(length) << 12) | u32::from(displacement);
      body.extend_from_slice(&word.to_be_bytes());
    }
  }
  Ok(())
}

fn hash_at(data: &[u8], position: usize) -> usize {
  let first = usize::from(data[position]);
  let second = usize::from(data[position + 1]);
  let third = usize::from(data[position + 2]);
  ((first << 10) ^ (second << 5) ^ third) & (HASH_SIZE - 1)
}

fn insert(
  data: &[u8],
  head: &mut [u32],
  previous: &mut [u32],
  position: usize,
) -> Result<(), CodecError> {
  if position + MIN_MATCH > data.len() {
    return Ok(());
  }
  // Positions live in 32-bit chain slots. `compress` rejects a stream larger
  // than the 24-bit header can declare, so this only guards the conversion.
  let index = u32::try_from(position)
    .map_err(|_| CodecError::Message("LZSS cannot index a stream this large".to_owned()))?;
  let hash = hash_at(data, position);
  previous[position] = head[hash];
  head[hash] = index;
  Ok(())
}

/// Finds the longest previous match at `position`.
///
/// Ties prefer the smaller displacement, matching nlzss's candidate ordering.
fn find_match(
  data: &[u8],
  head: &[u32],
  previous: &[u32],
  position: usize,
  format: Format,
) -> Option<(usize, usize)> {
  if position + MIN_MATCH > data.len() {
    return None;
  }
  let limit = format.max_match().min(data.len() - position);
  let mut candidate = head[hash_at(data, position)];
  let mut best: Option<(usize, usize)> = None;
  let mut examined = 0usize;

  while candidate != NONE && examined < CHAIN_LIMIT {
    let index = candidate as usize;
    let displacement = position - index;
    if displacement > WINDOW_SIZE {
      break;
    }

    let mut length = 0usize;
    while length < limit && data[index + length] == data[position + length] {
      length += 1;
    }
    if length >= MIN_MATCH {
      let better = match best {
        None => true,
        Some((best_length, best_displacement)) => {
          length > best_length || (length == best_length && displacement < best_displacement)
        }
      };
      if better {
        best = Some((length, displacement));
      }
    }
    if length == limit {
      break;
    }

    candidate = previous[index];
    examined += 1;
  }

  best
}
