use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::snes::common::BANK_SIZE;
use crate::codecs::nintendo::snes::common::LOROM_HEADER_OFFSET;
use crate::codecs::nintendo::snes::common::SnesRomMapType;
use crate::codecs::nintendo::snes::common::rom_banks::BankNumbers;
use crate::codecs::nintendo::snes::common::rom_banks::SnesCodecArgs;
use crate::codecs::nintendo::snes::common::rom_banks::SnesRomExtractor;
use log::*;

const CODEC_ID: &str = "std/nintendo/snes/cart/sa1";
const CODEC_NAME: &str = "SNES SA-1 ROM";
const CODEC_DESC: &str = r#"SNES SA-1 ROM codec

Separates a Super Nintendo Entertainment System/Super Famicom SA-1 ROM into
64 KiB banks, plus a copier header if present. Reassembles those banks into
the image they were split from, recalculating the SNES checksum of its header.

Arguments (JSON):
  bank_numbers (optional)
    mapped      Use mapped SNES bank numbers
    sequential  Use sequential physical bank numbers (default)

Example:
  {"bank_numbers":"mapped"}
"#;

pub(crate) struct SnesSa1Rom;

impl Codec for SnesSa1Rom {
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
    super::common::can_handle(data, LOROM_HEADER_OFFSET, &SnesRomMapType::SA1)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("rom_banks".to_string())
  }

  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    trace!("Decoding SA-1 ROM");
    let mut results: Vec<DecodedArtifact> = vec![];

    let mut extractor = SnesRomExtractor::new(data, BANK_SIZE * 2);
    if let Some(copier_header) = extractor.extract_copier_header()? {
      trace!("  Found copier header");
      let result = DecodedArtifact {
        name: "copier_header.bin".to_owned(),
        data: copier_header.to_vec(),
      };
      results.push(result);
    }
    let bank_count = extractor.banks_left();
    if bank_count > 64 {
      return Err(CodecError::Message(format!(
        "Too many ROM banks: {} (Max for HiROM: 64)",
        bank_count
      )));
    }

    let args = SnesCodecArgs::from_args(args)?;
    let mut rom_bank = match args.bank_numbers {
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
    trace!("Encoding SA-1 ROM");
    let args = SnesCodecArgs::from_args(args)?;
    // The numbering `decode` used, in the order it walked the banks.
    let groups = match args.bank_numbers {
      BankNumbers::Mapped => [(0xc0, 64)],
      BankNumbers::Sequential => [(0x00, 64)],
    };

    super::common::rom_banks::encode_rom_banks(
      artifacts,
      BANK_SIZE * 2,
      &groups,
      LOROM_HEADER_OFFSET,
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::nintendo::snes::common::testkit;

  /// The arguments asking for the numbering the console sees.
  const MAPPED: &str = r#"{"bank_numbers":"mapped"}"#;

  /// The mapping mode byte of a SA-1 running fast.
  const MAP_MODE: u8 = 0x23;

  /// Two banks rather than one, so a rebuild that dropped a bank, repeated one,
  /// or put two of them back in the wrong order could not pass.
  fn banks() -> Vec<Vec<u8>> {
    (0..2)
      .map(|index| testkit::bank(index, BANK_SIZE * 2))
      .collect()
  }

  /// An image of [`banks`] carrying the header a SA-1 keeps.
  fn image_with_header(copier: bool) -> Vec<u8> {
    testkit::image_with_header(copier, &banks(), LOROM_HEADER_OFFSET, MAP_MODE)
  }

  #[test]
  fn rebuilds_a_headerless_image_from_its_banks() {
    let image = image_with_header(false);

    assert_eq!(testkit::round_trip(&SnesSa1Rom, &image, None), image);
  }

  #[test]
  fn rebuilds_an_image_carrying_a_copier_header() {
    let image = image_with_header(true);

    assert_eq!(testkit::round_trip(&SnesSa1Rom, &image, None), image);
  }

  #[test]
  fn finalizes_the_header_of_a_rebuilt_image() {
    testkit::assert_finalizes_header(&SnesSa1Rom, &banks(), LOROM_HEADER_OFFSET, MAP_MODE);
  }

  #[test]
  fn finalizes_the_header_of_a_rebuild_numbered_as_mapped_banks() {
    let image = image_with_header(true);
    let artifacts = SnesSa1Rom.decode(&image, Some(MAPPED)).unwrap();

    let rebuilt = SnesSa1Rom
      .encode(Vec::new(), &artifacts, Some(MAPPED))
      .unwrap();

    assert_eq!(rebuilt, image);
    testkit::assert_valid_header(&SnesSa1Rom, &rebuilt, LOROM_HEADER_OFFSET);
  }

  #[test]
  fn rebuilds_the_banks_numbered_as_mapped_banks() {
    let args = MAPPED;
    let image = image_with_header(true);
    let artifacts = SnesSa1Rom.decode(&image, Some(args)).unwrap();

    assert_eq!(
      testkit::names(&artifacts),
      [
        "copier_header.bin",
        "rom_bank_c0.bin",
        "rom_bank_c1.bin",
      ]
    );
    assert_eq!(
      SnesSa1Rom
        .encode(Vec::new(), &artifacts, Some(args))
        .unwrap(),
      image
    );
  }
}
