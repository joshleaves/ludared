use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::snes::common::EXHIROM_HEADER_OFFSET;
use crate::codecs::nintendo::snes::common::SnesRomMapType;
use log::*;

const CODEC_ID: &str = "std/nintendo/snes/cart/exhirom";
const CODEC_NAME: &str = "SNES ExHiROM";
const CODEC_DESC: &str = r#"SNES ExHiROM codec

Separates a Super Nintendo Entertainment System/Super Famicom ExHiROM into
64 KiB banks, plus a copier header if present. Reassembles those banks into
the image they were split from, recalculating the SNES checksum of its header.

Arguments (JSON):
  bank_numbers (optional)
    mapped      Use mapped SNES bank numbers
    sequential  Use sequential physical bank numbers (default)

Example:
  {"bank_numbers":"mapped"}
"#;

pub(crate) struct SnesExHiRom;

impl Codec for SnesExHiRom {
  fn id(&self) -> &str {
    CODEC_ID
  }
  fn name(&self) -> &str {
    CODEC_NAME
  }
  fn description(&self) -> &str {
    CODEC_DESC
  }

  fn kind(&self) -> CodecKind {
    CodecKind::Rebuild
  }

  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    super::common::can_handle(data, EXHIROM_HEADER_OFFSET, &SnesRomMapType::ExHiROM)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("rom_banks".to_string())
  }

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding SNES ExHiROM");
    super::common::rom_banks::extract_extended_rom_banks(data, args)
  }

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"encode"))
  )]
  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    trace!("Encoding SNES ExHiROM");
    super::common::rom_banks::encode_extended_rom_banks(artifacts, args, EXHIROM_HEADER_OFFSET)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::nintendo::snes::common::BANK_SIZE;
  use crate::codecs::nintendo::snes::common::testkit;

  /// The arguments asking for the numbering the console sees.
  const MAPPED: &str = r#"{"bank_numbers":"mapped"}"#;

  /// The mapping mode byte of an ExHiROM running fast.
  const MAP_MODE: u8 = 0x25;

  /// The banks an image of this codec carries. An ExHiROM keeps its header at
  /// $40ffc0, so an image has to be big enough to reach one with its banks: 128
  /// of them, which is also the most the extended layout numbers.
  const BANKS: usize = 128;

  /// Enough banks that a rebuild that dropped one, repeated one, or put two of
  /// them back in the wrong order could not pass.
  fn banks() -> Vec<Vec<u8>> {
    (0..BANKS)
      .map(|index| testkit::bank(index, BANK_SIZE * 2))
      .collect()
  }

  /// An image of [`banks`] carrying the header an ExHiROM keeps.
  fn image_with_header(copier: bool) -> Vec<u8> {
    testkit::image_with_header(copier, &banks(), EXHIROM_HEADER_OFFSET, MAP_MODE)
  }

  #[test]
  fn rebuilds_a_headerless_image_from_its_banks() {
    let image = image_with_header(false);

    assert_eq!(testkit::round_trip(&SnesExHiRom, &image, None), image);
  }

  #[test]
  fn rebuilds_an_image_carrying_a_copier_header() {
    let image = image_with_header(true);

    assert_eq!(testkit::round_trip(&SnesExHiRom, &image, None), image);
  }

  #[test]
  fn finalizes_the_header_of_a_rebuilt_image() {
    testkit::assert_finalizes_header(&SnesExHiRom, &banks(), EXHIROM_HEADER_OFFSET, MAP_MODE);
  }

  #[test]
  fn finalizes_the_header_of_a_rebuild_numbered_as_mapped_banks() {
    let image = image_with_header(true);
    let artifacts = SnesExHiRom.decode(&image, Some(MAPPED)).unwrap();

    let rebuilt = SnesExHiRom
      .encode(Vec::new(), &artifacts, Some(MAPPED))
      .unwrap();

    assert_eq!(rebuilt, image);
    testkit::assert_valid_header(&SnesExHiRom, &rebuilt, EXHIROM_HEADER_OFFSET);
  }

  #[test]
  fn rebuilds_the_banks_numbered_as_mapped_banks() {
    let args = MAPPED;
    let image = image_with_header(true);
    let artifacts = SnesExHiRom.decode(&image, Some(args)).unwrap();
    let names = testkit::names(&artifacts);

    // Mapped numbering turns back halfway through: the second group of banks is
    // numbered below the first, so sorting by name would reverse half the image.
    assert_eq!(names.len(), BANKS + 1);
    assert_eq!(names[0], "copier_header.bin");
    assert_eq!(names[1], "rom_bank_c0.bin");
    assert_eq!(names[1 + 64], "rom_bank_40.bin");
    assert_eq!(names[BANKS], "rom_bank_7f.bin");

    assert_eq!(
      SnesExHiRom
        .encode(Vec::new(), &artifacts, Some(args))
        .unwrap(),
      image
    );
  }

  #[test]
  fn numbers_a_short_image_the_way_the_console_sees_it() {
    // An image too short to hold the header this codec keeps is still one this
    // codec can cut up, and the numbering is the same one a long image gets.
    let image = testkit::image(None, &banks()[..3]);
    let artifacts = SnesExHiRom.decode(&image, Some(MAPPED)).unwrap();

    assert_eq!(
      testkit::names(&artifacts),
      [
        "rom_bank_c0.bin",
        "rom_bank_c1.bin",
        "rom_bank_c2.bin",
      ]
    );
  }

  #[test]
  fn refuses_an_image_too_small_to_carry_its_own_header() {
    // The one image this codec cannot rebuild: its header sits at $40ffc0, and
    // three banks do not reach that far.
    let image = testkit::image(None, &banks()[..3]);
    let artifacts = SnesExHiRom.decode(&image, None).unwrap();

    assert!(
      SnesExHiRom
        .encode(Vec::new(), &artifacts, None)
        .unwrap_err()
        .to_string()
        .contains("no room for the internal header"),
      "an ExHiROM shorter than its header must not be rebuilt into one"
    );
  }
}
