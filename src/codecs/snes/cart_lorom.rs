use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::BANK_SIZE;
use crate::codecs::snes::common::LOROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;
use crate::codecs::snes::common::rom_banks::BankNumbers;
use crate::codecs::snes::common::rom_banks::SnesCodecArgs;
use crate::codecs::snes::common::rom_banks::SnesRomExtractor;
use log::*;

const CODEC_ID: &str = "std/nintendo/snes/cart/lorom";
const CODEC_NAME: &str = "SNES LoROM";
const CODEC_DESC: &str = "SNES LoROM codec

Separates a Super Nintendo Entertainment System/Super Famicom LoROM into 32 KiB banks, plus a copier header if present. Reassembles those banks into the image they were split from, recalculating the SNES checksum of its header.

Available arguments:
  bank_numbers
    mapped      Use mapped SNES bank numbers
    sequential  Use sequential physical bank numbers (default)
";

/// The SNES LoROM codec.
///
/// A rebuild puts the banks back where they belong and then finalises the
/// internal header: the checksum and its complement cover the image they are
/// written into, so they only describe it once it is whole. An image that
/// already carried a valid checksum therefore comes back byte for byte, and one
/// that did not has those four bytes and nothing else rewritten.
pub(crate) struct SnesLoRom;

impl Codec for SnesLoRom {
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
    super::common::can_handle(data, LOROM_HEADER_OFFSET, &SnesRomMapType::LoROM)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("rom_banks".to_string())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding LoROM");
    let mut results: Vec<DecodedArtifact> = vec![];

    let mut extractor = SnesRomExtractor::new(data, BANK_SIZE);
    if let Some(copier_header) = extractor.extract_copier_header()? {
      trace!("  Found copier header");
      results.push(DecodedArtifact {
        name: "copier_header.bin".to_owned(),
        data: copier_header.to_vec(),
      });
    }
    let bank_count = extractor.banks_left();
    if bank_count > 128 {
      return Err(CodecError::Message(format!(
        "Too many ROM banks: {} (Max for LoROM: 128)",
        bank_count
      )));
    }

    let args = SnesCodecArgs::from_args(args)?;
    let mut rom_bank = match args.bank_numbers {
      BankNumbers::Mapped => 0x80,
      BankNumbers::Sequential => 0x00,
    };

    extractor.extract_rom_banks(128)?.iter().for_each(|bytes| {
      // trace!(format!("  Found ROM bank {:02x}", rom_bank))
      results.push(DecodedArtifact {
        name: format!("rom_bank_{:02x}.bin", rom_bank),
        data: bytes.to_vec(),
      });
      rom_bank += 1;
    });

    Ok(results)
  }

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding LoROM");
    let args = SnesCodecArgs::from_args(args)?;
    // The numbering `decode` used, in the order it walked the banks.
    let groups = match args.bank_numbers {
      BankNumbers::Mapped => [(0x80, 128)],
      BankNumbers::Sequential => [(0x00, 128)],
    };

    super::common::rom_banks::encode_rom_banks(artifacts, BANK_SIZE, &groups, LOROM_HEADER_OFFSET)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::snes::common::checksum::SnesChecksum;
  use crate::codecs::snes::common::testkit;
  use crate::testing::fixtures::lorom;

  /// The arguments asking for the numbering the console sees.
  const MAPPED: &str = r#"{"bank_numbers":"mapped"}"#;

  /// The mapping mode byte of a LoROM running fast.
  const MAP_MODE: u8 = 0x20;

  /// The article title the fixture embeds, rewritten the way a first build
  /// would rewrite it. The same length as the one it replaces, since a bank has
  /// to stay the size the image is made of.
  const RETITLE: &[u8] = b"Smashing The Stack For Ludared Rules!";

  /// Three banks rather than one, so a rebuild that dropped a bank, repeated
  /// one, or put two of them back in the wrong order could not pass.
  fn banks() -> Vec<Vec<u8>> {
    (0..3)
      .map(|index| testkit::bank(index, BANK_SIZE))
      .collect()
  }

  /// An image of [`banks`] carrying the header a LoROM keeps.
  fn image_with_header(copier: bool) -> Vec<u8> {
    testkit::image_with_header(copier, &banks(), LOROM_HEADER_OFFSET, MAP_MODE)
  }

  /// Where the fixture embeds the article title the manifest nests.
  fn title_offset() -> usize {
    lorom::SOURCE
      .windows(lorom::TITLE.len())
      .position(|window| window == lorom::TITLE)
      .expect("the fixture should embed the article the manifest nests")
  }

  /// Rebuilds the real fixture from its banks with the article in the first one
  /// retitled.
  fn rebuild_with_new_title() -> Vec<u8> {
    assert_eq!(
      RETITLE.len(),
      lorom::TITLE.len(),
      "a retitled article is rewritten in place"
    );

    let title = title_offset();
    let mut artifacts = SnesLoRom.decode(lorom::SOURCE, None).unwrap();

    let bank = artifacts
      .iter_mut()
      .find(|artifact| artifact.name == "rom_bank_00.bin")
      .expect("the fixture should decode into banks");
    bank.data[title..title + RETITLE.len()].copy_from_slice(RETITLE);

    SnesLoRom.encode(Vec::new(), &artifacts, None).unwrap()
  }

  #[test]
  fn rebuilds_a_headerless_image_from_its_banks() {
    let image = image_with_header(false);

    assert_eq!(testkit::round_trip(&SnesLoRom, &image, None), image);
  }

  #[test]
  fn rebuilds_an_image_carrying_a_copier_header() {
    let image = image_with_header(true);

    assert_eq!(testkit::round_trip(&SnesLoRom, &image, None), image);
  }

  #[test]
  fn rebuilds_the_real_lorom_fixture() {
    for args in [None, Some(MAPPED)] {
      assert_eq!(
        testkit::round_trip(&SnesLoRom, lorom::SOURCE, args),
        lorom::SOURCE,
        "round trip failed with {args:?}"
      );
    }
  }

  #[test]
  fn finalizes_the_header_of_a_rebuilt_image() {
    testkit::assert_finalizes_header(&SnesLoRom, &banks(), LOROM_HEADER_OFFSET, MAP_MODE);
  }

  #[test]
  fn reads_the_checksum_of_the_real_fixture() {
    // A known value, from a real LoROM: the fixture's header carries the sum of
    // its own contents at $ffde and the opposite of it at $ffdc.
    let image = image_with_header(false);

    assert_eq!(
      SnesChecksum::checksum(lorom::SOURCE, 0, 0x10000, SnesRomMapType::LoROM),
      0xd07b
    );
    assert_eq!(
      testkit::fields(lorom::SOURCE, LOROM_HEADER_OFFSET),
      (0x2f84, 0xd07b),
      "the fixture should carry the checksum and complement of its contents"
    );
    assert_eq!(
      SnesChecksum::checksum(&image, 0, 0x20000, SnesRomMapType::LoROM),
      testkit::fields(&image, LOROM_HEADER_OFFSET).1,
      "a stamped image should carry the checksum of its own contents"
    );
  }

  #[test]
  fn counts_the_rom_and_not_the_copier_header_leading_it() {
    // A copier header is a file format of its own, sitting in front of the ROM
    // rather than being part of it, and the checksum is a checksum of the ROM.
    // The fixture is used here because the bank data of the test kit sums to
    // nothing at all, which would hide a copier header counted or not.
    let mut image = testkit::copier_header();
    image.extend_from_slice(lorom::SOURCE);

    let rebuilt = testkit::round_trip(&SnesLoRom, &image, None);

    // The ROM behind the header is the fixture, checksum and all, so a rebuild
    // of it has to write that very checksum back.
    assert_eq!(
      testkit::fields(&rebuilt, LOROM_HEADER_OFFSET),
      testkit::fields(lorom::SOURCE, LOROM_HEADER_OFFSET),
      "a copier header should not be counted in the checksum"
    );
    testkit::assert_valid_header(&SnesLoRom, &rebuilt, LOROM_HEADER_OFFSET);
  }

  #[test]
  fn rebuilds_a_changed_bank_with_a_checksum_that_follows_the_change() {
    let title = title_offset();
    let rebuilt = rebuild_with_new_title();

    // A checksum is a sum, so the bytes a bank gives up and takes are worth
    // exactly as much again in it.
    let moved: i32 = RETITLE
      .iter()
      .zip(lorom::TITLE)
      .map(|(after, before)| *after as i32 - *before as i32)
      .sum();

    assert_eq!(
      &rebuilt[title..title + RETITLE.len()],
      RETITLE,
      "the rebuild should hold the new article"
    );
    assert_eq!(
      testkit::fields(&rebuilt, LOROM_HEADER_OFFSET).1,
      testkit::fields(lorom::SOURCE, LOROM_HEADER_OFFSET)
        .1
        .wrapping_add(moved as u16),
      "the checksum should have moved by what the change was worth"
    );
    testkit::assert_valid_header(&SnesLoRom, &rebuilt, LOROM_HEADER_OFFSET);
  }

  #[test]
  fn rewrites_nothing_but_the_changed_bytes_and_the_checksum() {
    let title = title_offset();
    let rebuilt = rebuild_with_new_title();
    let header = LOROM_HEADER_OFFSET;

    let mut expected: Vec<usize> = RETITLE
      .iter()
      .zip(lorom::TITLE)
      .enumerate()
      .filter(|(_, (after, before))| after != before)
      .map(|(offset, _)| title + offset)
      .collect();
    for field in 0x1c..0x20 {
      if lorom::SOURCE[header + field] != rebuilt[header + field] {
        expected.push(header + field);
      }
    }

    let changed: Vec<usize> = (0..lorom::SOURCE.len())
      .filter(|offset| lorom::SOURCE[*offset] != rebuilt[*offset])
      .collect();

    assert_eq!(
      rebuilt.len(),
      lorom::SOURCE.len(),
      "a rebuild should be the size of the image it came from"
    );
    assert_eq!(
      changed, expected,
      "only the retitled bytes and the checksum should differ"
    );
  }

  #[test]
  fn rebuilds_the_banks_numbered_as_mapped_banks() {
    let image = image_with_header(true);
    let artifacts = SnesLoRom.decode(&image, Some(MAPPED)).unwrap();

    // The banks come back named the way the console sees them rather than the
    // way they sit in the image, so the rebuild has to recover the physical
    // order from the name alone.
    assert_eq!(
      testkit::names(&artifacts),
      [
        "copier_header.bin",
        "rom_bank_80.bin",
        "rom_bank_81.bin",
        "rom_bank_82.bin",
      ]
    );
    assert_eq!(
      SnesLoRom
        .encode(Vec::new(), &artifacts, Some(MAPPED))
        .unwrap(),
      image
    );
  }

  #[test]
  fn finalizes_the_header_of_a_rebuild_numbered_as_mapped_banks() {
    // The numbering is not the physical order, so a rebuild that finalises the
    // header of banks it put in the wrong place would still carry a checksum of
    // an image nobody had.
    let image = image_with_header(true);
    let artifacts = SnesLoRom.decode(&image, Some(MAPPED)).unwrap();

    let rebuilt = SnesLoRom
      .encode(Vec::new(), &artifacts, Some(MAPPED))
      .unwrap();

    assert_eq!(rebuilt, image);
    testkit::assert_valid_header(&SnesLoRom, &rebuilt, LOROM_HEADER_OFFSET);
  }

  #[test]
  fn slices_the_banks_out_in_the_order_they_sit_in_the_image() {
    // The mapped numbering happens to run in file order here, so the numbering
    // and the physical order cannot be told apart from one test alone: what
    // matters is that decoding hands the banks over in the order it cut them.
    for args in [None, Some(MAPPED)] {
      let image = image_with_header(true);
      let artifacts = SnesLoRom.decode(&image, args).unwrap();

      testkit::assert_physical_order(&artifacts, &image);
    }
  }

  #[test]
  fn rebuilds_the_image_without_reading_a_parent_it_was_handed() {
    // Every bank of the image is held by an artifact, so a parent handed to a
    // rebuild codec makes no difference at all: not its contents, and not its
    // length, which the image is in no way obliged to match.
    let image = image_with_header(true);
    let artifacts = SnesLoRom.decode(&image, None).unwrap();
    let unrelated = vec![0xa5; image.len() * 3 + 7];

    assert_eq!(
      SnesLoRom.encode(Vec::new(), &artifacts, None).unwrap(),
      image
    );
    assert_eq!(
      SnesLoRom.encode(unrelated, &artifacts, None).unwrap(),
      image,
      "a rebuild codec must not read the parent it is handed"
    );
  }
}
