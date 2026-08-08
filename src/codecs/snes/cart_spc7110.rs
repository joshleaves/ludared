use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::snes::common::HIROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;

const CODEC_ID: &str = "std/nintendo/snes/cart/spc7110";
const CODEC_NAME: &str = "SNES SPC7110 ROM";
const CODEC_DESC: &str = "SNES SPC7110 ROM extractor

Separates a SPC7110 ROM into 64Kib banks.
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
}
