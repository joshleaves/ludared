use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::snes::common::LOROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;

const CODEC_ID: &str = "std/nintendo/snes/cart/sa1";
const CODEC_NAME: &str = "SNES SA-1 ROM";
const CODEC_DESC: &str = "SNES SA-1 ROM extractor

Separates a SA-1 ROM into 64Kib banks.
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
}
