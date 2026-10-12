use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::BANK_SIZE;
use crate::codecs::snes::common::LOROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;
use crate::codecs::snes::common::extractor::BankNumbers;
use crate::codecs::snes::common::extractor::SnesCodecArgs;
use crate::codecs::snes::common::extractor::SnesRomExtractor;
use log::*;

const CODEC_ID: &str = "std/nintendo/snes/cart/lorom";
const CODEC_NAME: &str = "SNES LoROM";
const CODEC_DESC: &str = "SNES LoROM codec

Separates a Super Nintendo Entertainment System/Super Famicom LoROM into 32 KiB banks, plus a copier header if present. Reassembles those banks into the image they were split from.

Available arguments:
  bank_numbers
    mapped      Use mapped SNES bank numbers
    sequential  Use sequential physical bank numbers (default)
";

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

    super::common::extractor::encode_rom_banks(artifacts, BANK_SIZE, &groups)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::snes::common::testkit;
  use crate::testing::fixtures::lorom;

  /// Three banks rather than one, so a rebuild that dropped a bank, repeated
  /// one, or put two of them back in the wrong order could not pass.
  fn banks() -> Vec<Vec<u8>> {
    (0..3)
      .map(|index| testkit::bank(index, BANK_SIZE))
      .collect()
  }

  #[test]
  fn rebuilds_a_headerless_image_from_its_banks() {
    let image = testkit::image(None, &banks());

    assert_eq!(testkit::round_trip(&SnesLoRom, &image, None), image);
  }

  #[test]
  fn rebuilds_an_image_carrying_a_copier_header() {
    let image = testkit::image(Some(testkit::copier_header()), &banks());

    assert_eq!(testkit::round_trip(&SnesLoRom, &image, None), image);
  }

  #[test]
  fn rebuilds_the_real_lorom_fixture() {
    for args in [
      None,
      Some(r#"{"bank_numbers":"mapped"}"#),
    ] {
      assert_eq!(
        testkit::round_trip(&SnesLoRom, lorom::SOURCE, args),
        lorom::SOURCE,
        "round trip failed with {args:?}"
      );
    }
  }

  #[test]
  fn rebuilds_the_banks_numbered_as_mapped_banks() {
    let args = r#"{"bank_numbers":"mapped"}"#;
    let image = testkit::image(Some(testkit::copier_header()), &banks());
    let artifacts = SnesLoRom.decode(&image, Some(args)).unwrap();

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
        .encode(Vec::new(), &artifacts, Some(args))
        .unwrap(),
      image
    );
  }

  #[test]
  fn slices_the_banks_out_in_the_order_they_sit_in_the_image() {
    // The mapped numbering happens to run in file order here, so the numbering
    // and the physical order cannot be told apart from one test alone: what
    // matters is that decoding hands the banks over in the order it cut them.
    for args in [
      None,
      Some(r#"{"bank_numbers":"mapped"}"#),
    ] {
      let image = testkit::image(Some(testkit::copier_header()), &banks());
      let artifacts = SnesLoRom.decode(&image, args).unwrap();

      testkit::assert_physical_order(&artifacts, &image);
    }
  }

  #[test]
  fn rebuilds_the_image_without_reading_a_parent_it_was_handed() {
    // Every bank of the image is held by an artifact, so a parent handed to a
    // rebuild codec makes no difference at all: not its contents, and not its
    // length, which the image is in no way obliged to match.
    let image = testkit::image(Some(testkit::copier_header()), &banks());
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
