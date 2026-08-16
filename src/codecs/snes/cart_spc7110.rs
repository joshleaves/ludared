use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::HIROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;
use log::*;

const CODEC_ID: &str = "std/nintendo/snes/cart/spc7110";
const CODEC_NAME: &str = "SNES SPC7110 ROM";
const CODEC_DESC: &str = "SNES SPC7110 ROM decoder

Separates a Super Nintendo Entertainment System/Super Famicom SPC7110 ROM into 64 KiB banks, plus a copier header if present.

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

  fn can_handle(&self, data: &[u8]) -> CodecHandlingConfidence {
    super::common::can_handle(data, HIROM_HEADER_OFFSET, &SnesRomMapType::SPC7110)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("rom_banks".to_string())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding SNES SPC7110 ROM");
    super::common::extractor::extract_extended_rom_banks(data, args)
  }
}
