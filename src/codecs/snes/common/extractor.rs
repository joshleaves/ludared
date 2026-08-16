use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BankNumbers {
  Mapped,
  #[default]
  Sequential,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SnesCodecArgs {
  #[serde(default)]
  pub(crate) bank_numbers: BankNumbers,
}

impl SnesCodecArgs {
  pub(crate) fn from_args(args: Option<&str>) -> Result<Self, serde_json::Error> {
    match args {
      None => Ok(Self::default()),
      Some(s) => serde_json::from_str(s),
    }
  }
}

pub struct SnesRomExtractor<'a> {
  data: &'a [u8],
  bank_size: usize,
  position: usize,
}

impl<'a> SnesRomExtractor<'a> {
  pub fn new(data: &'a [u8], bank_size: usize) -> Self {
    let position = 0;
    Self {
      data,
      bank_size,
      position,
    }
  }

  pub fn banks_left(&self, bank_size: usize) -> usize {
    (self.data.len() - self.position) / bank_size
  }

  pub fn extract_copier_header(&mut self) -> Result<Option<&[u8]>, CodecError> {
    match common::rom_layout(self.data) {
      common::RomLayout::Invalid => {
        Err(CodecError::Message("Invalid copier header size".to_owned()))
      }
      common::RomLayout::Headerless => Ok(None),
      common::RomLayout::CopierHeader => {
        self.position = common::COPIER_HEADER_SIZE;
        Ok(Some(&self.data[..common::COPIER_HEADER_SIZE]))
      }
    }
  }

  pub fn extract_rom_banks(&mut self, mut bank_count: usize) -> Result<Vec<&[u8]>, CodecError> {
    let mut results: Vec<&[u8]> = vec![];
    while bank_count != 0
      && let Some(bytes) = self.data.get(self.position..self.position + self.bank_size)
    {
      results.push(bytes);
      bank_count -= 1;
      self.position += self.bank_size;
    }
    Ok(results)
  }
}

pub fn extract_extended_rom_banks(
  data: &[u8],
  args: Option<&str>,
) -> Result<Vec<DecodedArtifact>, CodecError> {
  let mut results: Vec<DecodedArtifact> = vec![];
  let mut extractor = SnesRomExtractor::new(data, common::BANK_SIZE * 2);
  if let Some(copier_header) = extractor.extract_copier_header()? {
    // trace!("  Found copier header")
    let result = DecodedArtifact {
      name: "copier_header.bin".to_owned(),
      data: copier_header.to_vec(),
    };
    results.push(result);
  }
  let bank_count = extractor.banks_left(common::BANK_SIZE * 2);
  if bank_count > 130 {
    return Err(CodecError::Message(format!(
      "Too many ROM banks: {} (Maximum: 130)",
      bank_count
    )));
  }

  let args = SnesCodecArgs::from_args(args)?;
  let mut rom_bank = match args.bank_numbers {
    BankNumbers::Mapped => 0xC0,
    BankNumbers::Sequential => 0x00,
  };

  extractor.extract_rom_banks(64)?.iter().for_each(|bytes| {
    results.push(DecodedArtifact {
      name: format!("rom_bank_{:02x}.bin", rom_bank),
      data: bytes.to_vec(),
    });
    rom_bank += 1;
  });

  rom_bank = match args.bank_numbers {
    BankNumbers::Mapped => 0x40,
    BankNumbers::Sequential => rom_bank,
  };

  extractor.extract_rom_banks(64)?.iter().for_each(|bytes| {
    results.push(DecodedArtifact {
      name: format!("rom_bank_{:02x}.bin", rom_bank),
      data: bytes.to_vec(),
    });
    rom_bank += 1;
  });

  rom_bank = match args.bank_numbers {
    BankNumbers::Mapped => 0x3E,
    BankNumbers::Sequential => rom_bank,
  };

  extractor.extract_rom_banks(2)?.iter().for_each(|bytes| {
    results.push(DecodedArtifact {
      name: format!("rom_bank_{:02x}.bin", rom_bank),
      data: bytes.to_vec(),
    });
    rom_bank += 1;
  });

  Ok(results)
}
