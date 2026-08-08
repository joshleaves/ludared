use crate::codecs::snes::common::SnesRomMapType;

/// For reference implementation, check ghidra-snes on GH:
/// - https://github.com/joshleaves/ghidra-snes/blob/master/src/main/java/ghidra_snes/common/rom/Checksum.java
pub struct SnesChecksum;

impl SnesChecksum {
  pub fn checksum(
    data: &[u8],
    offset: usize,
    declared_size: usize,
    rom_type: SnesRomMapType,
  ) -> u16 {
    let Some(rom) = data.get(offset..) else {
      return 0;
    };

    if rom.is_empty() {
      return 0;
    }

    match rom_type {
      SnesRomMapType::SPC7110 => Self::checksum_spc7110(rom, declared_size),
      _ => {
        let mapped_size = std::cmp::max(rom.len(), declared_size);
        Self::checksum_mapped_rom(rom, mapped_size)
      }
    }
  }

  fn checksum_mapped_rom(rom: &[u8], mapped_size: usize) -> u16 {
    let mut image = rom.to_vec();

    while image.len() < mapped_size {
      let mirror_start = mapped_size >> 1;

      if mirror_start >= image.len() {
        break;
      }

      let mirror_length = std::cmp::min(mapped_size - image.len(), image.len() - mirror_start);

      let previous_length = image.len();

      image.resize(previous_length + mirror_length, 0);
      image.copy_within(mirror_start..mirror_start + mirror_length, previous_length);
    }

    Self::checksum_bytes(&image)
  }

  fn checksum_spc7110(rom: &[u8], declared_size: usize) -> u16 {
    let checksum = Self::checksum_bytes(rom);

    if rom.len() == 0x00300000 && declared_size > rom.len() {
      return checksum.wrapping_mul(2);
    }

    checksum
  }

  fn checksum_bytes(bytes: &[u8]) -> u16 {
    bytes
      .iter()
      .fold(0u16, |checksum, &value| checksum.wrapping_add(value as u16))
  }
}
