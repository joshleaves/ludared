use crate::codecs::snes::common::SnesRomMapType;

/// Where the two fields of the checksum sit in the internal header, which every
/// mapping mode shares: the complement at `$ffdc` and the checksum at `$ffde`,
/// which is `$1c` and `$1e` counted from the start of the header. They stand
/// next to each other, the complement first.
const FIELDS_OFFSET: usize = 0x1c;

/// The SNES checksum: the 16-bit sum of every byte of a ROM, taken over the
/// image as the console maps it.
///
/// The procedure is the one the SNES development manual describes and the
/// SNESdev wiki spells out, and which [`write_header`] carries out. Two sources
/// back the details this file encodes, and neither is code copied from either:
///
/// - The mapped-ROM mirroring of a file that is not the size its header
///   declares. For reference implementation, check ghidra-snes on GH:
///   <https://github.com/joshleaves/ghidra-snes/blob/master/src/main/java/ghidra_snes/common/rom/Checksum.java>
///   and the format itself at <https://snes.nesdev.org/wiki/ROM_header>.
///
/// - The retail mirroring of SPC7110 cartridges, a 3 MiB ROM on a 4 MiB cart
///   being read through twice over. These are Optiroc's observations in
///   SuperFamicheck, MIT licensed, at <https://github.com/Optiroc/SuperFamicheck>,
///   reached through the reference implementation above.
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

  /// Recalculates the checksum of a rebuilt image and writes it, along with its
  /// complement, into the internal header at `rom_header_offset`.
  ///
  /// `rom_header_offset` is where the header sits within the ROM itself; an
  /// image carrying a copier header is accounted for the same way the decoders
  /// account for the header they read.
  ///
  /// The checksum is a sum over every byte of the image as the console maps it,
  /// the two fields it is stored in included, so they are filled in beforehand
  /// with `$ffff` and `$0000`: those two are each other's complement and so count
  /// exactly as much as the pair that replaces them, which leaves the checksum a
  /// true sum of the finished image.
  ///
  /// Returns the checksum written, or [`None`] when the image is too short, or
  /// laid out wrongly, to hold the header.
  ///
  /// [`None`]: Option::None
  pub fn write_header(image: &mut [u8], rom_header_offset: usize) -> Option<u16> {
    let rom_offset = match super::rom_layout(image) {
      super::RomLayout::Headerless => 0,
      super::RomLayout::CopierHeader => super::COPIER_HEADER_SIZE,
      super::RomLayout::Invalid => return None,
    };

    let header_offset = rom_offset + rom_header_offset;
    let header = super::parse_snes_header(image, header_offset)?;
    let declared_size = header.rom_size();
    let rom_type = header.rom_map_type();

    // The header is whole from here on, so the four bytes its two fields take
    // up are within the image as well.
    let fields = header_offset + FIELDS_OFFSET..header_offset + FIELDS_OFFSET + 4;
    image[fields.clone()].copy_from_slice(&[
      0xff,
      0xff,
      0x00,
      0x00,
    ]);

    let checksum = Self::checksum(image, rom_offset, declared_size, rom_type);

    let mut written = [0u8; 4];
    written[..2].copy_from_slice(&(!checksum).to_le_bytes());
    written[2..].copy_from_slice(&checksum.to_le_bytes());
    image[fields].copy_from_slice(&written);

    Some(checksum)
  }

  /// The sum of an image mirrored up to the size the console maps it as.
  ///
  /// A file that is not a power of two is a larger region followed by a smaller
  /// one, and the console maps the smaller over and over until the two fill the
  /// size the header declares.
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

  /// The sum of an SPC7110 ROM, which the console maps by repetition rather than
  /// by mirroring: a 3 MiB ROM on a 4 MiB cart is read through twice over, and
  /// any other size is read once. See [`SnesChecksum`] for where that behaviour
  /// was observed.
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
