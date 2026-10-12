use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::BANK_SIZE;
use crate::codecs::snes::common::HIROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;
use crate::codecs::snes::common::extractor::BankNumbers;
use crate::codecs::snes::common::extractor::SnesCodecArgs;
use crate::codecs::snes::common::extractor::SnesRomExtractor;
use log::*;

const CODEC_ID: &str = "std/nintendo/snes/cart/hirom";
const CODEC_NAME: &str = "SNES HiROM";
const CODEC_DESC: &str = "SNES HiROM codec

Separates a Super Nintendo Entertainment System/Super Famicom HiROM into 64 KiB banks, plus a copier header if present. Reassembles those banks into the image they were split from.

Available arguments:
  bank_numbers
    mapped      Use mapped SNES bank numbers
    sequential  Use sequential physical bank numbers (default)
";

pub(crate) struct SnesHiRom;

impl Codec for SnesHiRom {
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
    super::common::can_handle(data, HIROM_HEADER_OFFSET, &SnesRomMapType::HiROM)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("rom_banks".to_string())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decodig HiROM");
    let mut results: Vec<DecodedArtifact> = vec![];

    let mut extractor = SnesRomExtractor::new(data, BANK_SIZE * 2);
    if let Some(copier_header) = extractor.extract_copier_header()? {
      trace!("  Found copier header");
      let result = DecodedArtifact {
        name: "copier_header.bin".to_owned(),
        data: copier_header.to_vec(),
      };
      results.push(result);
    }
    let bank_count = extractor.banks_left();
    if bank_count > 64 {
      return Err(CodecError::Message(format!(
        "Too many ROM banks: {} (Max for HiROM: 64)",
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

    Ok(results)
  }

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding HiROM");
    let args = SnesCodecArgs::from_args(args)?;
    // The numbering `decode` used, in the order it walked the banks.
    let groups = match args.bank_numbers {
      BankNumbers::Mapped => [(0xc0, 64)],
      BankNumbers::Sequential => [(0x00, 64)],
    };

    super::common::extractor::encode_rom_banks(artifacts, BANK_SIZE * 2, &groups)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::snes::common::testkit;

  /// Two banks rather than one, so a rebuild that dropped a bank, repeated one,
  /// or put two of them back in the wrong order could not pass.
  fn banks() -> Vec<Vec<u8>> {
    (0..2)
      .map(|index| testkit::bank(index, BANK_SIZE * 2))
      .collect()
  }

  #[test]
  fn rebuilds_a_headerless_image_from_its_banks() {
    let image = testkit::image(None, &banks());

    assert_eq!(testkit::round_trip(&SnesHiRom, &image, None), image);
  }

  #[test]
  fn rebuilds_an_image_carrying_a_copier_header() {
    let image = testkit::image(Some(testkit::copier_header()), &banks());

    assert_eq!(testkit::round_trip(&SnesHiRom, &image, None), image);
  }

  #[test]
  fn rebuilds_the_banks_numbered_as_mapped_banks() {
    let args = r#"{"bank_numbers":"mapped"}"#;
    let image = testkit::image(Some(testkit::copier_header()), &banks());
    let artifacts = SnesHiRom.decode(&image, Some(args)).unwrap();

    assert_eq!(
      testkit::names(&artifacts),
      [
        "copier_header.bin",
        "rom_bank_c0.bin",
        "rom_bank_c1.bin",
      ]
    );
    assert_eq!(
      SnesHiRom
        .encode(Vec::new(), &artifacts, Some(args))
        .unwrap(),
      image
    );
  }
}
