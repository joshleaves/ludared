use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::snes::common::HIROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;

const CODEC_ID: &str = "std/nintendo/snes/cart/hirom";
const CODEC_NAME: &str = "SNES HiROM";
const CODEC_DESC: &str = "SNES HiROM extractor

Separates a HiROM into 64Kib banks.
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

  fn can_handle(&self, data: &[u8]) -> CodecHandlingConfidence {
    super::common::can_handle(data, HIROM_HEADER_OFFSET, &SnesRomMapType::HiROM)
  }
}
