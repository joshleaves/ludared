//! Building SNES images for the codec tests.
//!
//! Round-tripping needs images holding several banks, a copier header, or both,
//! in the numbering each codec hands out. Assembling them here keeps a codec's
//! test about the one thing that codec is responsible for.

use crate::codecs::Codec;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;

use super::COPIER_HEADER_SIZE;

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
