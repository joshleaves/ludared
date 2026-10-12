use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::HIROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;
use log::*;

const CODEC_ID: &str = "std/nintendo/snes/cart/spc7110";
const CODEC_NAME: &str = "SNES SPC7110 ROM";
const CODEC_DESC: &str = "SNES SPC7110 ROM codec

Separates a Super Nintendo Entertainment System/Super Famicom SPC7110 ROM into 64 KiB banks, plus a copier header if present. Reassembles those banks into the image they were split from, recalculating the SNES checksum of its header.

Available arguments:
  bank_numbers
    mapped      Use mapped SNES bank numbers
    sequential  Use sequential physical bank numbers (default)
";

pub(crate) struct SnesSpc7110Rom;

impl Codec for SnesSpc7110Rom {
  fn id(&self) -> &'static str {
    CODEC_ID
  }
  fn name(&self) -> &'static str {
    CODEC_NAME
  }
  fn description(&self) -> &'static str {
    CODEC_DESC
  }

  fn kind(&self) -> CodecKind {
    CodecKind::Rebuild
  }

  fn can_handle(&self, data: &[u8]) -> CodecHandlingConfidence {
    super::common::can_handle(data, HIROM_HEADER_OFFSET, &SnesRomMapType::SPC7110)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("rom_banks".to_string())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding SNES SPC7110 ROM");
    super::common::rom_banks::extract_extended_rom_banks(data, args)
  }

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding SNES SPC7110 ROM");
    super::common::rom_banks::encode_extended_rom_banks(artifacts, args, HIROM_HEADER_OFFSET)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::snes::common::BANK_SIZE;
  use crate::codecs::snes::common::testkit;

  /// The arguments asking for the numbering the console sees.
  const MAPPED: &str = r#"{"bank_numbers":"mapped"}"#;

  /// The mapping mode byte of a SPC7110 running fast.
  const MAP_MODE: u8 = 0x2a;

  /// Three banks rather than one, so a rebuild that dropped a bank, repeated
  /// one, or put two of them back in the wrong order could not pass.
  fn banks() -> Vec<Vec<u8>> {
    (0..3)
      .map(|index| testkit::bank(index, BANK_SIZE * 2))
      .collect()
  }

  /// An image of [`banks`] carrying the header a SPC7110 keeps.
  fn image_with_header(copier: bool) -> Vec<u8> {
    testkit::image_with_header(copier, &banks(), HIROM_HEADER_OFFSET, MAP_MODE)
  }

  #[test]
  fn rebuilds_a_headerless_image_from_its_banks() {
    let image = image_with_header(false);

    assert_eq!(testkit::round_trip(&SnesSpc7110Rom, &image, None), image);
  }

  #[test]
  fn rebuilds_an_image_carrying_a_copier_header() {
    let image = image_with_header(true);

    assert_eq!(testkit::round_trip(&SnesSpc7110Rom, &image, None), image);
  }

  #[test]
  fn finalizes_the_header_of_a_rebuilt_image() {
    testkit::assert_finalizes_header(&SnesSpc7110Rom, &banks(), HIROM_HEADER_OFFSET, MAP_MODE);
  }

  #[test]
  fn finalizes_the_header_of_a_rebuild_numbered_as_mapped_banks() {
    let image = image_with_header(true);
    let artifacts = SnesSpc7110Rom.decode(&image, Some(MAPPED)).unwrap();

    let rebuilt = SnesSpc7110Rom
      .encode(Vec::new(), &artifacts, Some(MAPPED))
      .unwrap();

    assert_eq!(rebuilt, image);
    testkit::assert_valid_header(&SnesSpc7110Rom, &rebuilt, HIROM_HEADER_OFFSET);
  }

  #[test]
  fn rebuilds_the_banks_numbered_as_mapped_banks() {
    let args = MAPPED;
    let image = image_with_header(true);
    let artifacts = SnesSpc7110Rom.decode(&image, Some(args)).unwrap();

    assert_eq!(
      testkit::names(&artifacts),
      [
        "copier_header.bin",
        "rom_bank_c0.bin",
        "rom_bank_c1.bin",
        "rom_bank_c2.bin",
      ]
    );
    assert_eq!(
      SnesSpc7110Rom
        .encode(Vec::new(), &artifacts, Some(args))
        .unwrap(),
      image
    );
  }

  /// The size of the retail SPC7110 ROM whose checksum is not the plain sum of
  /// its bytes: Tengai Makyou Zero ships as a 3 MiB ROM on a 4 MiB cart, and the
  /// console reads the whole of it twice over.
  const DOUBLED_ROM_SIZE: usize = 0x00300000;

  /// `size` bytes worth of banks.
  fn banks_of(size: usize) -> Vec<Vec<u8>> {
    (0..size / (BANK_SIZE * 2))
      .map(|index| testkit::bank(index, BANK_SIZE * 2))
      .collect()
  }

  /// An SPC7110 image of `size` bytes, declaring the cart it sits in.
  fn spc7110_rom(size: usize) -> Vec<u8> {
    testkit::image_with_header(false, &banks_of(size), HIROM_HEADER_OFFSET, MAP_MODE)
  }

  /// The plain sum of every byte of `image`, which is what every mapping but
  /// this one calls a checksum.
  fn plain_sum(image: &[u8]) -> u16 {
    image
      .iter()
      .fold(0u16, |sum, &byte| sum.wrapping_add(byte as u16))
  }

  #[test]
  fn counts_a_three_megabyte_rom_the_console_reads_twice() {
    // A 3 MiB ROM is declared as the 4 MiB cart holding it, and that gap is what
    // asks for its sum to be taken twice.
    let image = spc7110_rom(DOUBLED_ROM_SIZE);
    assert_eq!(image.len(), DOUBLED_ROM_SIZE);

    let rebuilt = testkit::round_trip(&SnesSpc7110Rom, &image, None);

    assert_eq!(
      testkit::fields(&rebuilt, HIROM_HEADER_OFFSET).1,
      plain_sum(&rebuilt).wrapping_mul(2),
      "a 3 MiB ROM declaring more than it holds is checksummed twice over"
    );
    assert_eq!(
      rebuilt, image,
      "an image already carrying that checksum should come back unchanged"
    );
    testkit::assert_valid_header(&SnesSpc7110Rom, &rebuilt, HIROM_HEADER_OFFSET);
  }

  #[test]
  fn counts_a_two_megabyte_rom_once() {
    // The doubling belongs to the 3 MiB dump alone: a ROM smaller than it, on a
    // cart of its own, is still a plain sum.
    let image = spc7110_rom(DOUBLED_ROM_SIZE / 3 * 2);
    let rebuilt = testkit::round_trip(&SnesSpc7110Rom, &image, None);

    assert_eq!(
      testkit::fields(&rebuilt, HIROM_HEADER_OFFSET).1,
      plain_sum(&rebuilt),
      "a ROM declaring exactly what it holds is a plain sum, whatever its size"
    );
    assert_eq!(rebuilt, image);
    testkit::assert_valid_header(&SnesSpc7110Rom, &rebuilt, HIROM_HEADER_OFFSET);
  }

  #[test]
  fn carries_a_change_through_a_doubled_rom_twice() {
    // The doubling carries what a rebuild changes through the checksum twice
    // over, where a ROM of any other size carries it once.
    let image = spc7110_rom(DOUBLED_ROM_SIZE);
    let mut artifacts = SnesSpc7110Rom.decode(&image, None).unwrap();
    let bank = artifacts
      .iter_mut()
      .find(|artifact| artifact.name.starts_with("rom_bank_"))
      .expect("the image should decode into banks");

    let (was, now) = (bank.data[0x40], bank.data[0x40].wrapping_add(1));
    bank.data[0x40] = now;

    let rebuilt = SnesSpc7110Rom.encode(Vec::new(), &artifacts, None).unwrap();
    let moved = (now.wrapping_sub(was)) as u16;

    assert_eq!(
      testkit::fields(&rebuilt, HIROM_HEADER_OFFSET).1,
      testkit::fields(&image, HIROM_HEADER_OFFSET)
        .1
        .wrapping_add(moved.wrapping_mul(2)),
      "a change should move a doubled checksum by twice its worth"
    );
    assert_eq!(
      plain_sum(&rebuilt).wrapping_sub(plain_sum(&image)),
      moved,
      "and the image itself should have changed by exactly the byte it was given"
    );
    testkit::assert_valid_header(&SnesSpc7110Rom, &rebuilt, HIROM_HEADER_OFFSET);
  }
}
