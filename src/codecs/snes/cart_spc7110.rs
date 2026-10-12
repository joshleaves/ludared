use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::snes::common::HIROM_HEADER_OFFSET;
use crate::codecs::snes::common::SnesRomMapType;
use log::*;

const CODEC_ID: &str = "std/nintendo/snes/cart/spc7110";
const CODEC_NAME: &str = "SNES SPC7110 ROM";
const CODEC_DESC: &str = "SNES SPC7110 ROM codec

Separates a Super Nintendo Entertainment System/Super Famicom SPC7110 ROM into 64 KiB banks, plus a copier header if present. Reassembles those banks into the image they were split from.

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

  fn kind(&self) -> CodecKind {
    CodecKind::Rebuild
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

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding SNES SPC7110 ROM");
    super::common::extractor::encode_extended_rom_banks(artifacts, args)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::snes::common::BANK_SIZE;
  use crate::codecs::snes::common::testkit;

  /// Three banks rather than one, so a rebuild that dropped a bank, repeated
  /// one, or put two of them back in the wrong order could not pass.
  fn banks() -> Vec<Vec<u8>> {
    (0..3)
      .map(|index| testkit::bank(index, BANK_SIZE * 2))
      .collect()
  }

  #[test]
  fn rebuilds_a_headerless_image_from_its_banks() {
    let image = testkit::image(None, &banks());

    assert_eq!(testkit::round_trip(&SnesSpc7110Rom, &image, None), image);
  }

  #[test]
  fn rebuilds_an_image_carrying_a_copier_header() {
    let image = testkit::image(Some(testkit::copier_header()), &banks());

    assert_eq!(testkit::round_trip(&SnesSpc7110Rom, &image, None), image);
  }

  #[test]
  fn rebuilds_the_banks_numbered_as_mapped_banks() {
    let args = r#"{"bank_numbers":"mapped"}"#;
    let image = testkit::image(Some(testkit::copier_header()), &banks());
    let artifacts = SnesSpc7110Rom.decode(&image, Some(args)).unwrap();

    assert_eq!(
      testkit::names(&artifacts),
      [
        "copier_header.bin",
        "rom_bank_c0.bin",
        "rom_bank_c1.bin",
        "rom_bank_c2.bin",
      ]
    );
    assert_eq!(
      SnesSpc7110Rom
        .encode(Vec::new(), &artifacts, Some(args))
        .unwrap(),
      image
    );
  }
}
