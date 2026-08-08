use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::snes::common::LOROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;

const CODEC_ID: &str = "std/nintendo/snes/cart/lorom";
const CODEC_NAME: &str = "SNES LoROM";
const CODEC_DESC: &str = "
SNES LoROM extractor

Separates a LoROM into 32Kib banks.
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
}
