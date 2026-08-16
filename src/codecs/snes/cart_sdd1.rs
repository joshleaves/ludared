use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::LOROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;
use log::*;

const CODEC_ID: &str = "std/nintendo/snes/cart/sdd1";
const CODEC_NAME: &str = "SNES SDD-1 ROM";
const CODEC_DESC: &str = "SNES SDD-1 ROM decoder

Separates a Super Nintendo Entertainment System/Super Famicom SDD-1 ROM into 64 KiB banks, plus a copier header if present.

Available arguments:
  bank_numbers
    mapped      Use mapped SNES bank numbers
    sequential  Use sequential physical bank numbers (default)
";

pub(crate) struct SnesSdd1Rom;

impl Codec for SnesSdd1Rom {
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
    super::common::can_handle(data, LOROM_HEADER_OFFSET, &SnesRomMapType::SDD1)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("rom_banks".to_string())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding SNES S-DD1 ROM");
    super::common::extractor::extract_extended_rom_banks(data, args)
  }
}
