use crate::codecs::CodecHandlingConfidence;
use checksum::SnesChecksum;
use log::*;

pub mod checksum;
pub mod extractor;

pub const BANK_SIZE: usize = 0x8000;
pub const COPIER_HEADER_SIZE: usize = 0x200;
pub const HEADER_SIZE: usize = 0x10000 - 0xffc0;
pub const LOROM_HEADER_OFFSET: usize = 0x7fc0;
pub const HIROM_HEADER_OFFSET: usize = 0xffc0;
pub const EXHIROM_HEADER_OFFSET: usize = 0x40ffc0;
const MAX_REASONABLE_ROM_SIZE: usize = 0x00c00000;

pub enum RomLayout {
  Headerless,
  CopierHeader,
  Invalid,
}

pub fn rom_layout(data: &[u8]) -> RomLayout {
  let copier_size = data.len() % BANK_SIZE;
  match copier_size {
    0 => RomLayout::Headerless,
    COPIER_HEADER_SIZE => RomLayout::CopierHeader,
    _ => {
      trace!("    Copier header: {:#06x}", copier_size);
      RomLayout::Invalid
    }
  }
}

pub struct SnesHeader<'a> {
  bytes: &'a [u8],
}

#[derive(PartialEq, Debug)]
pub enum SnesRomMapType {
  LoROM,
  HiROM,
  SDD1,
  SA1,
  ExHiROM,
  SPC7110,
  Unknown,
}

impl SnesHeader<'_> {
  pub fn checksum(&self) -> u16 {
    u16::from_le_bytes([
      self.bytes[0x1e],
      self.bytes[0x1f],
    ])
  }

  pub fn rom_map_type(&self) -> SnesRomMapType {
    match self.bytes[0x15] & 0x0f {
      0x0 => SnesRomMapType::LoROM,
      0x1 => SnesRomMapType::HiROM,
      0x2 => SnesRomMapType::SDD1,
      0x3 => SnesRomMapType::SA1,
      0x5 => SnesRomMapType::ExHiROM,
      0xA => SnesRomMapType::SPC7110,
      _ => SnesRomMapType::Unknown,
    }
  }

  pub fn rom_size(&self) -> usize {
    1024usize << self.bytes[0x17]
  }

  // Check 1: Header checksum + complement
  pub fn has_valid_checksum_complement(&self) -> bool {
    let complement = u16::from_le_bytes([
      self.bytes[0x1c],
      self.bytes[0x1d],
    ]);
    trace!("    Checksum:   {:#06x}", self.checksum());
    trace!("    Complement: {complement:#06x}");
    debug_result!(
      "Valid checksum complement" =>
      self.checksum() ^ complement == 0xffff
    )
  }

  // Check 2: Known RomMapType
  pub fn has_valid_rom_map_type(&self, rom_type: &SnesRomMapType) -> bool {
    trace!("    ROM map type:  {:?}", self.rom_map_type());
    trace!("    Expected type: {:?}", rom_type);
    debug_result!("Valid ROM type" => rom_type == &self.rom_map_type())
  }

  // Check 3: Valid ROM size
  // Check 4: No extravagant ROM size (12M)
  pub fn has_valid_rom_size(&self, data: &[u8], offset: usize) -> bool {
    trace!("    ROM real size:     {:>8}", data.len());
    trace!("    ROM declared size: {:>8}", self.rom_size());
    debug_result!("Valid ROM size" => self.rom_size() >= (data.len() - offset) && self.rom_size() < MAX_REASONABLE_ROM_SIZE)
  }

  // Check 5: Printable title
  pub fn has_ascii_title(&self) -> bool {
    let title = &self.bytes[0..0x15];
    trace!("    ROM title: {:?}", String::from_utf8_lossy(title));
    debug_result!("Valid ROM title" => title.iter().all(|byte| byte.is_ascii_graphic() || *byte == b' '))
  }

  // Check 6: Checksum verification against file data
  pub fn has_valid_checksum(&self, data: &[u8], offset: usize) -> bool {
    let real_checksum = SnesChecksum::checksum(data, offset, self.rom_size(), self.rom_map_type());
    trace!("    Checksum:      {:#06x}", self.checksum());
    trace!("    Data checksum: {real_checksum:#06x}");
    debug_result!("Valid checksum" => real_checksum == self.checksum())
  }
}

pub fn parse_snes_header(data: &[u8], offset: usize) -> Option<SnesHeader<'_>> {
  let bytes = data.get(offset..offset + HEADER_SIZE)?;
  Some(SnesHeader { bytes })
}

pub fn can_handle(
  data: &[u8],
  offset: usize,
  expected_type: &SnesRomMapType,
) -> CodecHandlingConfidence {
  let header_offset = match rom_layout(data) {
    RomLayout::Invalid => {
      debug_result!("Invalid copier header" => false);
      return CodecHandlingConfidence::No;
    }
    RomLayout::Headerless => offset,
    RomLayout::CopierHeader => offset + COPIER_HEADER_SIZE,
  };
  let Some(rom_header) = super::common::parse_snes_header(data, header_offset) else {
    debug_result!("Can extract a header" => false);
    return CodecHandlingConfidence::No;
  };

  let heuristics = [
    rom_header.has_valid_checksum_complement(),
    rom_header.has_valid_rom_map_type(expected_type),
    rom_header.has_valid_rom_size(data, header_offset - offset),
    rom_header.has_ascii_title(),
    rom_header.has_valid_checksum(data, header_offset - offset),
  ];

  let passed = heuristics.iter().filter(|result| **result).count();
  match passed {
    0 => CodecHandlingConfidence::No,
    1 => CodecHandlingConfidence::Possible,
    n if n == heuristics.len() => CodecHandlingConfidence::Certain,
    _ => CodecHandlingConfidence::Likely,
  }
}
