//! Building SNES images for the codec tests.
//!
//! Round-tripping needs images holding several banks, a copier header, or both,
//! in the numbering each codec hands out. Assembling them here keeps a codec's
//! test about the one thing that codec is responsible for.

use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;

use super::COPIER_HEADER_SIZE;
use super::checksum::SnesChecksum;
use super::parse_snes_header;
use super::rom_banks::ROM_BANK;

/// The title a stamped header carries, padded to the 21 bytes one keeps.
const TITLE: &[u8] = b"LUDARED TEST IMAGE   ";

/// A copier header, filled so that it cannot pass for a bank.
pub fn copier_header() -> Vec<u8> {
  (0..COPIER_HEADER_SIZE)
    .map(|offset| 0xc0u8.wrapping_add(offset as u8))
    .collect()
}

/// The `index`th bank of an image, `size` bytes long.
///
/// Every bank differs from every other one at every offset, so putting two
/// banks back in each other's place produces a different image rather than the
/// same one twice.
pub fn bank(index: usize, size: usize) -> Vec<u8> {
  (0..size)
    .map(|offset| (index as u8).wrapping_add(offset as u8))
    .collect()
}

/// An image laid out as an optional copier header followed by `banks`.
pub fn image(copier_header: Option<Vec<u8>>, banks: &[Vec<u8>]) -> Vec<u8> {
  let mut image = copier_header.unwrap_or_default();

  for bank in banks {
    image.extend_from_slice(bank);
  }

  image
}

/// An image of `banks`, carrying the internal header of the mapping it is of.
///
/// `header_offset` is where that mapping keeps its header and `map_mode` the
/// mapping it declares, which is what tells one codec's image from another's.
/// The checksum is stamped with the calculation the codecs finalise a rebuild
/// with, so a rebuild of this image hands back the image that went in.
///
/// The banks have to be long enough for the header to sit in them at all: a
/// HiROM header ends a 64 KiB bank, an ExHiROM one sits in the sixty-fifth.
pub fn image_with_header(
  copier: bool,
  banks: &[Vec<u8>],
  header_offset: usize,
  map_mode: u8,
) -> Vec<u8> {
  let mut image = image(copier.then(copier_header), banks);
  let header = header_at(&image, header_offset);

  assert!(
    header + 0x40 <= image.len(),
    "an image of {} bytes has no room for a header at {header_offset:#x}",
    image.len()
  );

  // A title the header can be read back out of, the mapping the image claims,
  // and the size the console maps it as.
  image[header..header + TITLE.len()].copy_from_slice(TITLE);
  image[header + 0x15] = map_mode;
  let declared = (image.len() - rom_offset(&image)).next_power_of_two();
  image[header + 0x17] = (declared / 1024).trailing_zeros() as u8;

  write_checksum(&mut image, header_offset);

  image
}

/// Where the ROM starts in `image`. A copier header leads it, and is not part of
/// the ROM: the checksum is a checksum of the ROM.
fn rom_offset(image: &[u8]) -> usize {
  image.len() % super::BANK_SIZE
}

/// Where the internal header of `image` sits in it, copier header or not.
fn header_at(image: &[u8], header_offset: usize) -> usize {
  header_offset + rom_offset(image)
}

/// Where the checksum and its complement sit in an internal header: `$ffdc` and
/// `$ffde`, or `$1c` and `$1e` counted from the start of the header.
const COMPLEMENT_AT: usize = 0x1c;
const CHECKSUM_AT: usize = 0x1e;

/// The complement and the checksum `image` carries in its internal header.
pub fn fields(image: &[u8], header_offset: usize) -> (u16, u16) {
  let header = header_at(image, header_offset);
  let read = |from: usize| {
    u16::from_le_bytes([
      image[header + from],
      image[header + from + 1],
    ])
  };

  (read(COMPLEMENT_AT), read(CHECKSUM_AT))
}

/// Writes the checksum of `image`, and its complement, into the header at
/// `header_offset`, the way a rebuild finalises it.
///
/// `$ffff` and `$0000` are each other's complement, so the four bytes they stand
/// in for count exactly as much as the pair that replaces them, which leaves the
/// checksum a true sum of the image it is written in.
pub fn write_checksum(image: &mut [u8], header_offset: usize) -> u16 {
  let header = header_at(image, header_offset);
  let (declared_size, rom_type) = {
    let parsed = parse_snes_header(image, header).expect("the image should hold a header");

    (parsed.rom_size(), parsed.rom_map_type())
  };

  image[header + COMPLEMENT_AT..header + CHECKSUM_AT + 2].copy_from_slice(&[
    0xff,
    0xff,
    0x00,
    0x00,
  ]);

  let checksum = SnesChecksum::checksum(image, rom_offset(image), declared_size, rom_type);
  let mut fields = [0u8; 4];
  fields[..2].copy_from_slice(&(!checksum).to_le_bytes());
  fields[2..].copy_from_slice(&checksum.to_le_bytes());
  image[header + COMPLEMENT_AT..header + CHECKSUM_AT + 2].copy_from_slice(&fields);

  checksum
}

/// Asserts that `image` carries the checksum and the complement the SNES header
/// calls for, and that `codec` still recognises what it rebuilt.
pub fn assert_valid_header(codec: &dyn Codec, image: &[u8], header_offset: usize) {
  let header = header_at(image, header_offset);
  let parsed = parse_snes_header(image, header).expect("the image should hold a header");
  let (complement, checksum) = fields(image, header_offset);

  assert_eq!(
    SnesChecksum::checksum(
      image,
      rom_offset(image),
      parsed.rom_size(),
      parsed.rom_map_type()
    ),
    checksum,
    "the header should carry the checksum of the image at {header:#x} it is written in"
  );
  assert_eq!(
    complement, !checksum,
    "the checksum and its complement at {header:#x} should be opposites"
  );
  assert_eq!(
    codec.can_handle(image).unwrap(),
    CodecHandlingConfidence::Certain,
    "a rebuilt image should pass every check the codec makes of one"
  );
}

/// Asserts that `rebuilt` is `image` with the byte at `changed` replaced, and
/// with the checksum fields beside it wherever they had to move to match.
fn assert_only_changed(image: &[u8], rebuilt: &[u8], changed: usize, header_offset: usize) {
  let header = header_at(image, header_offset);

  let mut expected = vec![changed];
  for field in COMPLEMENT_AT..=CHECKSUM_AT + 1 {
    if image[header + field] != rebuilt[header + field] {
      expected.push(header + field);
    }
  }

  let differing: Vec<usize> = (0..image.len())
    .filter(|offset| image[*offset] != rebuilt[*offset])
    .collect();

  assert_eq!(
    rebuilt.len(),
    image.len(),
    "a rebuild should not resize an image"
  );
  assert_eq!(
    differing, expected,
    "a rebuild should touch the byte it was given and the checksum, and nothing else"
  );
}

/// Runs the header finalization every SNES cartridge codec owes its image.
///
/// `header_offset` is where the codec finds the internal header and `map_mode`
/// the mapping its image declares: together they are how a codec tells its own
/// image from another's. A codec whose header sits too far into its image for
/// the banks to reach fails here, which is the point of saying so out loud.
pub fn assert_finalizes_header(
  codec: &dyn Codec,
  banks: &[Vec<u8>],
  header_offset: usize,
  map_mode: u8,
) {
  for copier in [false, true] {
    let image = image_with_header(copier, banks, header_offset, map_mode);

    // An image whose checksum is already valid comes back byte for byte: there
    // is nothing left for a rebuild to finalise.
    let rebuilt = round_trip(codec, &image, None);
    assert_eq!(
      rebuilt, image,
      "an image that was already valid should come back unchanged ({copier})"
    );
    assert_valid_header(codec, &rebuilt, header_offset);

    // One whose checksum was never filled in is given the pair it is missing,
    // rather than left counting a blank that stood in for it.
    let mut blank = image.clone();
    let header = header_at(&blank, header_offset);
    blank[header + COMPLEMENT_AT..header + CHECKSUM_AT + 2].fill(0);

    let rebuilt = round_trip(codec, &blank, None);
    assert_ne!(
      rebuilt, blank,
      "a blanked out checksum should not survive a rebuild ({copier})"
    );
    assert_valid_header(codec, &rebuilt, header_offset);

    // A changed bank moves the checksum by exactly what the change was worth,
    // and leaves every other byte of the image alone.
    let changed = rom_offset(&image) + 0x40;
    let mut artifacts = codec.decode(&image, None).unwrap();
    let bank = artifacts
      .iter_mut()
      .find(|artifact| artifact.name.starts_with(ROM_BANK))
      .expect("a codec should decode banks");
    let (before, after) = (bank.data[0x40], bank.data[0x40].wrapping_add(1));
    bank.data[0x40] = after;

    let rebuilt = codec.encode(Vec::new(), &artifacts, None).unwrap();
    assert_only_changed(&image, &rebuilt, changed, header_offset);
    assert_eq!(
      fields(&rebuilt, header_offset).1,
      fields(&image, header_offset)
        .1
        .wrapping_add((after.wrapping_sub(before)) as u16),
      "the checksum should have moved by what the change was worth ({copier})"
    );
    assert_valid_header(codec, &rebuilt, header_offset);
  }
}

/// Decodes `image` with `codec`, and encodes what came back straight away.
///
/// The parent handed to `encode` is whatever the codec's kind calls for: a
/// rebuild codec is given none, so a round trip that works here is one that
/// never reads the original back.
pub fn round_trip(codec: &dyn Codec, image: &[u8], args: Option<&str>) -> Vec<u8> {
  let artifacts = codec.decode(image, args).unwrap();

  let original: Vec<u8> = match codec.kind() {
    CodecKind::Rebuild => Vec::new(),
    CodecKind::Patch => image.to_vec(),
  };

  codec.encode(original, &artifacts, args).unwrap()
}

/// An artifact carrying `data` under `name`.
pub fn artifact(name: &str, data: &[u8]) -> DecodedArtifact {
  DecodedArtifact {
    name: name.to_owned(),
    data: data.to_vec(),
  }
}

/// The names of a set of artifacts, in the order they were handed over.
pub fn names(artifacts: &[DecodedArtifact]) -> Vec<&str> {
  artifacts
    .iter()
    .map(|artifact| artifact.name.as_str())
    .collect()
}

/// Asserts that the artifacts a decode handed over, read back in the order they
/// were given, are the image they were taken from.
///
/// Decoding slices an image in physical order whatever it numbers the pieces, so
/// this is the order an encoder has to put them back in. A codec numbering its
/// banks the way the console maps them still slices them in file order, and
/// sorting them by name on the way back would not recover it.
pub fn assert_physical_order(artifacts: &[DecodedArtifact], image: &[u8]) {
  let stitched: Vec<u8> = artifacts
    .iter()
    .flat_map(|artifact| artifact.data.iter().copied())
    .collect();

  assert_eq!(
    stitched, image,
    "decode should hand the pieces over in physical order"
  );
}

#[cfg(test)]
mod tests {
  use super::super::BANK_SIZE;
  use super::super::EXHIROM_HEADER_OFFSET;
  use super::super::HIROM_HEADER_OFFSET;
  use super::super::LOROM_HEADER_OFFSET;
  use super::*;

  /// The image each mapping is tested with, and the checksum it carries.
  ///
  /// The checksums were worked out from the procedure the SNES development
  /// manual describes and the SNESdev wiki spells out — fill the two fields with
  /// `$ffff` and `$0000`, add every byte of the image as the console maps it,
  /// keep the sum, and write it and its opposite back — rather than from the code
  /// that stamps them. Between them these cover the three cases the calculation
  /// has: an image declaring exactly what it holds (HiROM, SA-1, ExHiROM), one
  /// declaring more, and so mirrored up to it (LoROM, SDD-1), and the one mapping
  /// that maps no image at all (SPC7110).
  const REFERENCE_CHECKSUMS: [(&str, usize, usize, usize, u8, u16); 6] = [
    ("LoROM", 3, BANK_SIZE, LOROM_HEADER_OFFSET, 0x20, 0xf1b5),
    ("HiROM", 2, BANK_SIZE * 2, HIROM_HEADER_OFFSET, 0x21, 0xf1b6),
    ("SA-1", 2, BANK_SIZE * 2, LOROM_HEADER_OFFSET, 0x23, 0xf1b8),
    ("SDD-1", 3, BANK_SIZE * 2, LOROM_HEADER_OFFSET, 0x22, 0xf1b8),
    (
      "SPC7110",
      3,
      BANK_SIZE * 2,
      HIROM_HEADER_OFFSET,
      0x2a,
      0x71c0,
    ),
    (
      "ExHiROM",
      128,
      BANK_SIZE * 2,
      EXHIROM_HEADER_OFFSET,
      0x25,
      0x0600,
    ),
  ];

  #[test]
  fn stamps_the_checksums_the_format_calls_for() {
    for (mapping, count, bank_size, header_offset, map_mode, checksum) in REFERENCE_CHECKSUMS {
      let banks: Vec<Vec<u8>> = (0..count).map(|index| bank(index, bank_size)).collect();

      // A copier header leads the image without joining it, so it leaves the
      // checksum exactly where it was.
      for copier in [false, true] {
        let image = image_with_header(copier, &banks, header_offset, map_mode);

        assert_eq!(
          fields(&image, header_offset),
          (!checksum, checksum),
          "the {mapping} image should carry the checksum of its contents ({copier})"
        );
      }
    }
  }
}
