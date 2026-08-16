use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::EXHIROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;

const CODEC_ID: &str = "std/nintendo/snes/cart/exhirom";
const CODEC_NAME: &str = "SNES ExHiROM";
const CODEC_DESC: &str = "SNES ExHiROM extractor

Separates a ExHiROM into 64KiB banks.

Available arguments:
  bank_numbers
    mapped      Use mapped SNES bank numbers
    sequential  Use sequential physical bank numbers (default)
";

pub(crate) struct SnesExHiRom;

impl Codec for SnesExHiRom {
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
    super::common::can_handle(data, EXHIROM_HEADER_OFFSET, &SnesRomMapType::ExHiROM)
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    // trace!("Decoding SNES ExHiRom ROM");
    super::common::extractor::extract_extended_rom_banks(data, args)
  }
}
