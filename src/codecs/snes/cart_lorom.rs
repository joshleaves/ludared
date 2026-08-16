use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::BANK_SIZE;
use crate::codecs::snes::common::LOROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;
use crate::codecs::snes::common::extractor::BankNumbers;
use crate::codecs::snes::common::extractor::SnesCodecArgs;
use crate::codecs::snes::common::extractor::SnesRomExtractor;

const CODEC_ID: &str = "std/nintendo/snes/cart/lorom";
const CODEC_NAME: &str = "SNES LoROM";
const CODEC_DESC: &str = "SNES LoROM extractor

Separates a LoROM into 32KiB banks.

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

  fn can_handle(&self, data: &[u8]) -> CodecHandlingConfidence {
    super::common::can_handle(data, LOROM_HEADER_OFFSET, &SnesRomMapType::LoROM)
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    // trace!(format!("Decoding {}:", CODEC_ID))
    let mut results: Vec<DecodedArtifact> = vec![];

    let mut extractor = SnesRomExtractor::new(data, BANK_SIZE);
    if let Some(copier_header) = extractor.extract_copier_header()? {
      // trace!("  Found copier header")
      results.push(DecodedArtifact {
        name: "copier_header.bin".to_owned(),
        data: copier_header.to_vec(),
      });
    }
    let bank_count = extractor.banks_left(BANK_SIZE);
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
}
