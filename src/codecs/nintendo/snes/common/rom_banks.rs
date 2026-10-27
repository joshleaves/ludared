use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use crate::codecs::nintendo::snes::common;
use crate::codecs::nintendo::snes::common::checksum::SnesChecksum;

/// The artifact name every SNES decoder gives the copier header, when the image
/// carries one.
const COPIER_HEADER: &str = "copier_header.bin";

/// The prefix of every artifact name standing for a ROM bank.
pub(super) const ROM_BANK: &str = "rom_bank_";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BankNumbers {
  Mapped,
  #[default]
  Sequential,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SnesCodecArgs {
  #[serde(default)]
  pub(crate) bank_numbers: BankNumbers,
}

impl LudaredCodecArgs for SnesCodecArgs {}

pub struct SnesRomExtractor<'a> {
  data: &'a [u8],
  bank_size: usize,
  position: usize,
}

impl<'a> SnesRomExtractor<'a> {
  pub fn new(data: &'a [u8], bank_size: usize) -> Self {
    let position = 0;
    Self {
      data,
      bank_size,
      position,
    }
  }

  pub fn banks_left(&self) -> usize {
    (self.data.len() - self.position) / self.bank_size
  }

  pub fn extract_copier_header(&mut self) -> Result<Option<&[u8]>, CodecError> {
    match common::rom_layout(self.data) {
      common::RomLayout::Invalid => {
        Err(CodecError::Message("Invalid copier header size".to_owned()))
      }
      common::RomLayout::Headerless => Ok(None),
      common::RomLayout::CopierHeader => {
        self.position = common::COPIER_HEADER_SIZE;
        Ok(Some(&self.data[..common::COPIER_HEADER_SIZE]))
      }
    }
  }

  pub fn extract_rom_banks(&mut self, mut bank_count: usize) -> Result<Vec<&[u8]>, CodecError> {
    let mut results: Vec<&[u8]> = vec![];
    while bank_count != 0
      && let Some(bytes) = self.data.get(self.position..self.position + self.bank_size)
    {
      results.push(bytes);
      bank_count -= 1;
      self.position += self.bank_size;
    }
    Ok(results)
  }
}

pub fn extract_extended_rom_banks(
  data: &[u8],
  args: Option<&str>,
) -> Result<Vec<DecodedArtifact>, CodecError> {
  let mut results: Vec<DecodedArtifact> = vec![];
  let mut extractor = SnesRomExtractor::new(data, common::BANK_SIZE * 2);
  if let Some(copier_header) = extractor.extract_copier_header()? {
    // trace!("  Found copier header")
    let result = DecodedArtifact {
      name: "copier_header.bin".to_owned(),
      data: copier_header.to_vec(),
    };
    results.push(result);
  }
  let bank_count = extractor.banks_left();
  if bank_count > 130 {
    return Err(CodecError::Message(format!(
      "Too many ROM banks: {} (Maximum: 130)",
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

  rom_bank = match args.bank_numbers {
    BankNumbers::Mapped => 0x40,
    BankNumbers::Sequential => rom_bank,
  };

  extractor.extract_rom_banks(64)?.iter().for_each(|bytes| {
    results.push(DecodedArtifact {
      name: format!("rom_bank_{:02x}.bin", rom_bank),
      data: bytes.to_vec(),
    });
    rom_bank += 1;
  });

  rom_bank = match args.bank_numbers {
    BankNumbers::Mapped => 0x3E,
    BankNumbers::Sequential => rom_bank,
  };

  extractor.extract_rom_banks(2)?.iter().for_each(|bytes| {
    results.push(DecodedArtifact {
      name: format!("rom_bank_{:02x}.bin", rom_bank),
      data: bytes.to_vec(),
    });
    rom_bank += 1;
  });

  Ok(results)
}

/// Rebuilds the image [`extract_extended_rom_banks`] split, from the banks it
/// produced, and finalises the internal header at `rom_header_offset`.
///
/// The three groups the decoder numbers are the same ones, in the same order, so
/// a bank that is numbered after another is often the one that came first.
pub fn encode_extended_rom_banks(
  artifacts: &[DecodedArtifact],
  args: Option<&str>,
  rom_header_offset: usize,
) -> Result<Vec<u8>, CodecError> {
  let args = SnesCodecArgs::from_args(args)?;
  let groups = match args.bank_numbers {
    BankNumbers::Mapped => [
      (0xc0, 64),
      (0x40, 64),
      (0x3e, 2),
    ],
    BankNumbers::Sequential => [
      (0x00, 64),
      (0x40, 64),
      (0x80, 2),
    ],
  };

  let image = encode_rom_banks(artifacts, common::BANK_SIZE * 2, &groups, rom_header_offset)?;

  Ok(image)
}

/// Finalises the internal header of a rebuilt image, so that the checksum it
/// carries describes the image rather than the one the banks were cut from.
///
/// `rom_header_offset` is where the mapping the image is of puts that header.
/// An image too short, or laid out too oddly, to hold it cannot be finalised,
/// and an encoder has no business handing back an image claiming a checksum it
/// never computed.
fn finalize_header(image: &mut [u8], rom_header_offset: usize) -> Result<(), CodecError> {
  SnesChecksum::write_header(image, rom_header_offset)
    .map(|_| ())
    .ok_or_else(|| {
      CodecError::Message(format!(
        "Image of {} bytes has no room for the internal header at {rom_header_offset:#x}",
        image.len()
      ))
    })
}

/// The names a decoder numbers its banks with, in the order the banks sit in the
/// image.
///
/// `groups` lists the decoder's numbering as `(first bank number, bank count)`
/// pairs. Mapped numbering is not monotonic — an extended image numbers its
/// first group from `0xc0`, its second from `0x40` and its last from `0x3e` —
/// so the pairs carry the physical order explicitly rather than leaving it to be
/// sorted back out of the names.
fn bank_names(groups: &[(usize, usize)]) -> Vec<String> {
  groups
    .iter()
    .flat_map(|(first, count)| (0..*count).map(move |offset| bank_name(first + offset)))
    .collect()
}

/// The artifact name a decoder gives the bank numbered `number`.
fn bank_name(number: usize) -> String {
  format!("{ROM_BANK}{number:02x}.bin")
}

/// Rebuilds a SNES image from the artifacts a bank-splitting decoder produced,
/// and finalises the internal header at `rom_header_offset`.
///
/// `groups` is the numbering of the decoder the image came from and `bank_size`
/// the size of one of its banks; see [`bank_names`]. `rom_header_offset` is
/// where the mapping the image is of keeps its internal header, whose checksum
/// and complement are recalculated from the image as it now stands.
///
/// The copier header leads the image whenever the decoder emitted one, and the
/// banks follow in the order `groups` gives them. An image holds as many banks
/// as it has room for, so the banks present are a prefix of the numbering and
/// the first one missing ends the image. Every artifact has to be placed by
/// that walk: a bank the image skipped over, a name no bank of this codec
/// carries, and a second artifact of a name already given are all errors,
/// because each of them is a hole where a ROM should be.
pub fn encode_rom_banks(
  artifacts: &[DecodedArtifact],
  bank_size: usize,
  groups: &[(usize, usize)],
  rom_header_offset: usize,
) -> Result<Vec<u8>, CodecError> {
  let mut banks: BTreeMap<&str, &[u8]> = BTreeMap::new();

  for artifact in artifacts {
    if banks
      .insert(artifact.name.as_str(), artifact.data.as_slice())
      .is_some()
    {
      return Err(CodecError::DuplicateArtifact(artifact.name.clone()));
    }
  }

  let mut image: Vec<u8> = Vec::new();
  let mut placed: BTreeSet<&str> = BTreeSet::new();

  if let Some(header) = banks.get(COPIER_HEADER) {
    if header.len() != common::COPIER_HEADER_SIZE {
      return Err(CodecError::Message(format!(
        "Invalid copier header size: {} (Expected: {})",
        header.len(),
        common::COPIER_HEADER_SIZE
      )));
    }
    image.extend_from_slice(header);
    placed.insert(COPIER_HEADER);
  }

  let names = bank_names(groups);
  let numbered: BTreeSet<&str> = names.iter().map(String::as_str).collect();

  for name in &names {
    let Some(bank) = banks.get(name.as_str()) else {
      // The image ran out of banks. A bank this layout does number, sitting
      // beyond this one, was skipped rather than never reached, which is a hole
      // in the middle of a ROM. A bank the layout never numbered at all is not,
      // and the walk over the artifacts below reports it for what it is.
      if let Some(skipped) = banks
        .keys()
        .copied()
        .find(|entry| numbered.contains(entry) && !placed.contains(entry))
      {
        return Err(CodecError::Message(format!(
          "Missing ROM bank {name} before {skipped}"
        )));
      }
      break;
    };

    if bank.len() != bank_size {
      return Err(CodecError::Message(format!(
        "Invalid ROM bank {name}: {} bytes (Expected: {bank_size})",
        bank.len()
      )));
    }

    image.extend_from_slice(bank);
    placed.insert(name.as_str());
  }

  for artifact in artifacts {
    if !placed.contains(artifact.name.as_str()) {
      return Err(CodecError::Message(format!(
        "Unexpected artifact: {}",
        artifact.name
      )));
    }
  }

  // The checksum covers the image it is written into, so it is only final once
  // the banks are back where they belong.
  finalize_header(&mut image, rom_header_offset)?;

  Ok(image)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::nintendo::snes::common::testkit;

  /// The number of banks the extended layout numbers, and so the number an
  /// image has to hold before every group is used.
  const EXTENDED_BANKS: usize = 130;

  /// Where the stamped images keep their internal header, and the mapping mode
  /// they declare. These tests are about where banks go rather than about what
  /// any one mapping does, so the images are stamped as a plain HiROM.
  const HEADER_OFFSET: usize = common::HIROM_HEADER_OFFSET;
  const MAP_MODE: u8 = 0x21;

  /// A full image: enough banks that the numbered groups are all in play.
  ///
  /// The header is stamped so that a rebuild finalising it changes nothing, and
  /// these tests can go on comparing whole images byte for byte.
  fn full_image(copier_header: bool) -> Vec<u8> {
    let banks: Vec<Vec<u8>> = (0..EXTENDED_BANKS)
      .map(|index| testkit::bank(index, common::BANK_SIZE * 2))
      .collect();

    testkit::image_with_header(copier_header, &banks, HEADER_OFFSET, MAP_MODE)
  }

  #[test]
  fn rebuilds_a_full_image_numbered_sequentially() {
    let image = full_image(true);
    let artifacts = extract_extended_rom_banks(&image, None).unwrap();

    // Sequential numbering runs straight on, so the last bank of the image is
    // also the last one that was numbered.
    assert_eq!(
      testkit::names(&artifacts).last().copied(),
      Some(format!("rom_bank_{:02x}.bin", EXTENDED_BANKS - 1).as_str())
    );
    assert_eq!(
      encode_extended_rom_banks(&artifacts, None, HEADER_OFFSET).unwrap(),
      image
    );
  }

  #[test]
  fn rebuilds_the_mapped_groups_in_the_order_they_sit_in_the_image() {
    let args = r#"{"bank_numbers":"mapped"}"#;
    let image = full_image(true);
    let artifacts = extract_extended_rom_banks(&image, Some(args)).unwrap();
    let names = testkit::names(&artifacts);

    // Mapped numbering is not the physical order: the last two banks of the
    // image are numbered below the middle group rather than above it, so
    // putting the banks back in numerical order would reverse two thirds of
    // the image.
    assert_eq!(names.len(), EXTENDED_BANKS + 1);
    assert_eq!(names[0], "copier_header.bin");
    assert_eq!(names[1], "rom_bank_c0.bin");
    assert_eq!(names[1 + 64], "rom_bank_40.bin");
    assert_eq!(names[1 + 128], "rom_bank_3e.bin");
    assert_eq!(names[1 + 129], "rom_bank_3f.bin");

    assert_eq!(
      encode_extended_rom_banks(&artifacts, Some(args), HEADER_OFFSET).unwrap(),
      image
    );
  }

  #[test]
  fn slices_the_banks_out_in_the_order_they_sit_in_the_image() {
    // The numbering and the physical order disagree here: the last two banks of
    // a mapped image are numbered below the middle group, and a numbering of its
    // own would put them at the front. The bytes are cut in file order and handed
    // over in that order whatever they are called, which is what puts the image
    // back together.
    for args in [
      None,
      Some(r#"{"bank_numbers":"mapped"}"#),
    ] {
      let image = full_image(true);
      let artifacts = extract_extended_rom_banks(&image, args).unwrap();

      testkit::assert_physical_order(&artifacts, &image);
    }
  }

  #[test]
  fn rebuilds_an_image_of_a_single_bank() {
    let image = testkit::image_with_header(
      false,
      &[testkit::bank(
        0,
        common::BANK_SIZE * 2,
      )],
      HEADER_OFFSET,
      MAP_MODE,
    );

    assert_eq!(
      encode_extended_rom_banks(
        &extract_extended_rom_banks(&image, None).unwrap(),
        None,
        HEADER_OFFSET
      )
      .unwrap(),
      image
    );
  }

  #[test]
  fn rebuilds_an_image_of_no_banks_at_all() {
    // An image holding only a copier header decodes to that header alone, and
    // has nowhere to put an internal header of its own.
    let image = testkit::copier_header();
    let artifacts = extract_extended_rom_banks(&image, None).unwrap();

    assert_eq!(testkit::names(&artifacts), ["copier_header.bin"]);
    assert!(
      encode_extended_rom_banks(&artifacts, None, HEADER_OFFSET)
        .unwrap_err()
        .to_string()
        .contains("no room for the internal header"),
      "an image with no room for a header cannot be finalised"
    );
  }

  #[test]
  fn refuses_a_bank_the_image_cannot_hold() {
    let short = testkit::bank(0, common::BANK_SIZE * 2 - 1);
    let artifacts = [
      testkit::artifact("copier_header.bin", &testkit::copier_header()),
      testkit::artifact("rom_bank_00.bin", &short),
    ];

    assert!(
      encode_extended_rom_banks(&artifacts, None, HEADER_OFFSET)
        .unwrap_err()
        .to_string()
        .contains("Invalid ROM bank rom_bank_00.bin"),
      "a truncated bank should be refused"
    );
  }

  #[test]
  fn refuses_a_copier_header_of_the_wrong_size() {
    let artifacts = [testkit::artifact(
      "copier_header.bin",
      b"TOO SHORT",
    )];

    assert!(
      encode_extended_rom_banks(&artifacts, None, HEADER_OFFSET)
        .unwrap_err()
        .to_string()
        .contains("Invalid copier header size"),
      "a truncated copier header should be refused"
    );
  }

  #[test]
  fn refuses_a_bank_the_image_skipped() {
    let bank = testkit::bank(1, common::BANK_SIZE * 2);
    // The first bank of the image is absent, so the second cannot be placed.
    let artifacts = [testkit::artifact(
      "rom_bank_01.bin",
      &bank,
    )];

    assert!(
      encode_extended_rom_banks(&artifacts, None, HEADER_OFFSET)
        .unwrap_err()
        .to_string()
        .contains("Missing ROM bank rom_bank_00.bin before rom_bank_01.bin"),
      "a hole at the front of an image should be refused"
    );
  }

  #[test]
  fn refuses_an_artifact_no_bank_of_the_image_could_be() {
    let bank = testkit::bank(0, common::BANK_SIZE * 2);
    let artifacts = [
      testkit::artifact("rom_bank_00.bin", &bank),
      testkit::artifact("rom_bank_fe.bin", &bank),
    ];

    assert!(
      encode_extended_rom_banks(&artifacts, None, HEADER_OFFSET)
        .unwrap_err()
        .to_string()
        .contains("Unexpected artifact: rom_bank_fe.bin"),
      "a bank outside the numbering should be refused"
    );
  }

  #[test]
  fn refuses_two_artifacts_of_the_same_name() {
    let bank = testkit::bank(0, common::BANK_SIZE * 2);
    let artifacts = [
      testkit::artifact("rom_bank_00.bin", &bank),
      testkit::artifact("rom_bank_00.bin", &bank),
    ];

    assert!(
      matches!(
        encode_extended_rom_banks(&artifacts, None, HEADER_OFFSET),
        Err(CodecError::DuplicateArtifact(name)) if name == "rom_bank_00.bin"
      ),
      "two artifacts of one name should be refused"
    );
  }

  #[test]
  fn rebuilds_the_banks_whatever_order_they_arrive_in() {
    let image = full_image(false);
    let mut artifacts = extract_extended_rom_banks(&image, None).unwrap();
    artifacts.reverse();

    assert_eq!(
      encode_extended_rom_banks(&artifacts, None, HEADER_OFFSET).unwrap(),
      image,
      "artifacts are placed by name, not by position"
    );
  }
}
