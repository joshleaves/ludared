use serde::Deserialize;

use super::common_mode::CDROM_SECTOR_MAGIC;
use super::common_mode::MODE2;
use super::common_mode::looks_like_xa;
use super::common_mode::msf;
use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;

const CODEC_ID: &str = "std/cdrom/mode2";
const CODEC_NAME: &str = "UNTESTED: CD-ROM Mode 2 non-XA (2352-byte sectors)";
const CODEC_DESC: &str = r#"UNTESTED: Raw CD-ROM Mode 2 (non-XA) sector converter

WARNING: This codec was implemented along std/cdrom_mode_xa and is supposed to
work, according to the format specs, but has NEVER been tested properly. You can
use it at your own risk, and are welcome to provide feedback.

Extracts 2336 user bytes from each 2352-byte raw Mode 2 sector.
Rebuilding regenerates the sync pattern, MSF address, and mode byte.

This codec is for non-XA Mode 2 only. CD-ROM XA Form 1/Form 2 sectors
have their own subheaders and error protection and need a separate codec.
Non-XA Mode 2 has no EDC or ECC fields to regenerate.

Rebuilding generates canonical sector headers; original headers are not preserved.

Arguments (JSON):
  start_lba (default: 0)
    Logical block address of the first sector. MSF = LBA + 150 frames.

Example:
  {"start_lba":0}
"#;

const RAW_SECTOR_SIZE: usize = 2352;
const USER_SECTOR_SIZE: usize = 2336;
const LEAD_IN_FRAMES: u32 = 150;

pub(crate) struct Mode2;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mode2CodecArgs {
  #[serde(default)]
  pub start_lba: u32,
}

impl LudaredCodecArgs for Mode2CodecArgs {}

fn encode_sector(frame: u32, data: &[u8]) -> Result<[u8; RAW_SECTOR_SIZE], String> {
  let mut sector = [0u8; RAW_SECTOR_SIZE];
  sector[..12].copy_from_slice(&CDROM_SECTOR_MAGIC);
  sector[12..15].copy_from_slice(&msf(frame)?);
  sector[15] = MODE2;

  // User data.
  sector[16..].copy_from_slice(data);

  Ok(sector)
}

impl Codec for Mode2 {
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

  fn can_handle(&self, _data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    // Untested: disable automatic detection until the codec is properly validated.
    // Explicit use with --force is still supported.
    Ok(CodecHandlingConfidence::No)

    // if data.is_empty() || !data.len().is_multiple_of(RAW_SECTOR_SIZE) {
    //   return Ok(CodecHandlingConfidence::No);
    // }

    // for sector in data.as_chunks::<RAW_SECTOR_SIZE>().0 {
    //   if sector[..12] != CDROM_SECTOR_MAGIC || sector[15] != MODE2 || looks_like_xa(sector) {
    //     return Ok(CodecHandlingConfidence::No);
    //   }
    // }

    // Ok(CodecHandlingConfidence::Certain)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("mode2".to_owned())
  }

  /// Extracts the 2336-byte user data from each raw MODE2/2352 sector.
  ///
  /// Validates sector boundaries, synchronization patterns and mode bytes.
  ///
  /// Returns a single `data.bin` artifact.
  /// `start_lba` is accepted but does not affect extraction.
  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    let _args = Mode2CodecArgs::from_args(args)?;

    if !data.len().is_multiple_of(RAW_SECTOR_SIZE) {
      return Err(CodecError::Message(
        "Input size is not a multiple of 2352".into(),
      ));
    }

    let mut output = Vec::with_capacity(data.len() / RAW_SECTOR_SIZE * USER_SECTOR_SIZE);
    for (index, sector) in data.as_chunks::<RAW_SECTOR_SIZE>().0.iter().enumerate() {
      if !sector.starts_with(&CDROM_SECTOR_MAGIC) {
        return Err(CodecError::Message(format!(
          "Invalid sync pattern in sector {index}"
        )));
      }
      if sector[15] != MODE2 {
        return Err(CodecError::Message(format!(
          "Expected Mode 2 sector at {index}, found mode {}",
          sector[15]
        )));
      }
      if looks_like_xa(sector) {
        return Err(CodecError::Message(format!(
          "Sector {index} looks like CD-ROM XA; use codec 'std/cdrom/mode_xa' instead"
        )));
      }
      output.extend_from_slice(&sector[16..2352]);
    }

    Ok(vec![DecodedArtifact {
      name: "data.bin".to_owned(),
      data: output,
    }])
  }

  /// Rebuilds raw MODE2/2352 sectors from the `data.bin` artifact.
  ///
  /// Requires exactly one artifact containing a multiple of 2336 bytes.
  /// Generates sector headers.
  ///
  /// `start_lba` specifies the logical address of the first sector (default: 0).
  /// Sector MSF addresses are calculated by adding the standard 150-frame offset.
  ///
  /// The original input is ignored. Noncanonical sector metadata is not preserved.
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
    let args = Mode2CodecArgs::from_args(args)?;

    if artifacts.len() != 1 || artifacts[0].name != "data.bin" {
      return Err(CodecError::Message(
        "Expected exactly one artifact named 'data.bin'".into(),
      ));
    }

    let data = &artifacts[0].data;
    if !data.len().is_multiple_of(USER_SECTOR_SIZE) {
      return Err(CodecError::Message(
        "Input size is not a multiple of 2336".into(),
      ));
    }

    let mut output = Vec::with_capacity(data.len() / USER_SECTOR_SIZE * RAW_SECTOR_SIZE);

    for (sector_index, chunk) in data.as_chunks::<USER_SECTOR_SIZE>().0.iter().enumerate() {
      let sector_index =
        u32::try_from(sector_index).map_err(|_| CodecError::Message("Too many sectors".into()))?;
      let frame = args
        .start_lba
        .checked_add(sector_index)
        .and_then(|lba| lba.checked_add(LEAD_IN_FRAMES))
        .ok_or_else(|| CodecError::Message("Sector address overflow".into()))?;

      let sector = encode_sector(frame, chunk).map_err(CodecError::Message)?;
      output.extend_from_slice(&sector);
    }
    Ok(output)
  }
}

#[cfg(test)]
mod tests {
  use super::super::edc_ecc::edc;
  use super::*;

  // Commented until properly tested with real data
  // #[test]
  // fn roundtrip() {
  //   let codec = Mode2;
  //   let mut data = vec![0u8; USER_SECTOR_SIZE * 3];
  //   for (i, byte) in data.iter_mut().enumerate() {
  //     *byte = (i as u8).wrapping_mul(37).wrapping_add(13);
  //   }
  //   let artifacts = vec![DecodedArtifact {
  //     name: "data.bin".into(),
  //     data: data.clone(),
  //   }];
  //   let encoded = codec.encode(Vec::new(), &artifacts, None).unwrap();
  //   assert_eq!(
  //     codec.can_handle(&encoded).unwrap(),
  //     CodecHandlingConfidence::Certain
  //   );
  //   assert_eq!(codec.decode(&encoded, None).unwrap()[0].data, data);
  //   assert_eq!(
  //     &encoded[12..16],
  //     &[
  //       0x00,
  //       0x02,
  //       0x00,
  //       0x02
  //     ]
  //   );
  // }

  #[test]
  fn rejects_xa_and_mode1() {
    let codec = Mode2;
    let mut sector = [0u8; RAW_SECTOR_SIZE];
    sector[1..11].fill(0xFF);
    sector[15] = 2;
    sector[16..24].copy_from_slice(&[
      1, 2, 3, 4, 1, 2, 3, 4,
    ]);

    // XA Form 1 EDC.
    let crc = edc(&sector[16..2072]);
    sector[2072..2076].copy_from_slice(&crc.to_le_bytes());

    assert_eq!(
      codec.can_handle(&sector).unwrap(),
      CodecHandlingConfidence::No
    );
    assert!(codec.decode(&sector, None).is_err());

    sector[15] = 1;
    assert_eq!(
      codec.can_handle(&sector).unwrap(),
      CodecHandlingConfidence::No
    );
  }
}
