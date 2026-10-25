use super::PrsError;

pub(super) struct BitReader<'a> {
  data: &'a [u8],
  pos: usize,
  flags: u8,
  bits: u8,
}

impl<'a> BitReader<'a> {
  pub(super) fn new(data: &'a [u8]) -> Self {
    Self {
      data,
      pos: 0,
      flags: 0,
      bits: 0,
    }
  }

  pub(super) fn read_bit(&mut self) -> Result<u8, PrsError> {
    if self.bits == 0 {
      self.flags = self.read_byte()?;
      self.bits = 8;
    }

    let value = self.flags & 1;
    self.flags >>= 1;
    self.bits -= 1;

    Ok(value)
  }

  pub(super) fn read_byte(&mut self) -> Result<u8, PrsError> {
    let byte = *self.data.get(self.pos).ok_or(PrsError::UnexpectedEof)?;
    self.pos += 1;
    Ok(byte)
  }

  pub(super) fn read_word(&mut self) -> Result<u16, PrsError> {
    let lo = self.read_byte()? as u16;
    let hi = self.read_byte()? as u16;

    Ok(lo | (hi << 8))
  }
}

pub(super) fn prs_decompress(data: &[u8]) -> Result<Vec<u8>, PrsError> {
  let mut reader = BitReader::new(data);
  let mut out = Vec::new();

  loop {
    if reader.read_bit()? == 1 {
      out.push(reader.read_byte()?);
      continue;
    }

    let (length, distance) = if reader.read_bit()? == 1 {
      let value = reader.read_word()?;

      if value == 0 {
        break;
      }

      let size_bits = (value & 7) as usize;
      let distance = 8192 - ((value >> 3) as usize);

      let length = if size_bits == 0 {
        reader.read_byte()? as usize + 1
      } else {
        size_bits + 2
      };

      (length, distance)
    } else {
      let high = reader.read_bit()? as usize;
      let low = reader.read_bit()? as usize;

      let length = (low | (high << 1)) + 2;
      let distance = 256 - reader.read_byte()? as usize;

      (length, distance)
    };

    if distance == 0 || distance > out.len() {
      return Err(PrsError::InvalidBackReference);
    }

    for _ in 0..length {
      let byte = out[out.len() - distance];
      out.push(byte);
    }
  }

  Ok(out)
}
