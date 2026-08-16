use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::BANK_SIZE;
use crate::codecs::snes::common::LOROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;
use crate::codecs::snes::common::extractor::BankNumbers;
use crate::codecs::snes::common::extractor::SnesCodecOptions;
use crate::codecs::snes::common::extractor::SnesRomExtractor;

const CODEC_ID: &str = "std/nintendo/snes/cart/sa1";
const CODEC_NAME: &str = "SNES SA-1 ROM";
const CODEC_DESC: &str = "SNES SA-1 ROM extractor

Separates a SA-1 ROM into 64KiB banks.

Available options:
  bank_numbers
    mapped      Use mapped SNES bank numbers
    sequential  Use sequential physical bank numbers (default)
";

pub(crate) struct SnesSa1Rom;

impl Codec for SnesSa1Rom {
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
    super::common::can_handle(data, LOROM_HEADER_OFFSET, &SnesRomMapType::SA1)
  }

  fn decode(&self, data: &[u8], options: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    // trace!(format!("Decoding {}:", CODEC_ID))
    let mut results: Vec<DecodedArtifact> = vec![];

    let mut extractor = SnesRomExtractor::new(data, BANK_SIZE * 2);
    if let Some(copier_header) = extractor.extract_copier_header()? {
      // trace!("  Found copier header")
      let result = DecodedArtifact {
        name: "copier_header.bin".to_owned(),
        data: copier_header.to_vec(),
      };
      results.push(result);
    }
    let bank_count = extractor.banks_left(BANK_SIZE * 2);
    if bank_count > 64 {
      return Err(CodecError::Message(format!(
        "Too many ROM banks: {} (Max for HiROM: 64)",
        bank_count
      )));
    }

    let options = SnesCodecOptions::from_options(options)?;
    let mut rom_bank = match options.bank_numbers {
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
}
