use serde::Deserialize;

use super::CdRomFileSystem;
use super::common_mode::CDROM_SECTOR_MAGIC;
use super::common_mode::MODE1;
use super::common_mode::msf;
use super::edc_ecc::ecc_block;
use super::edc_ecc::ecc_tables;
use super::edc_ecc::edc;
use super::iso9660::Iso9660;
use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;

const CODEC_ID: &str = "std/cdrom/mode1";
const CODEC_NAME: &str = "CD-ROM Mode 1 (2352-byte sectors)";
const CODEC_DESC: &str = r#"Raw CD-ROM Mode 1 sector converter

Extracts the 2048-byte user data from each 2352-byte MODE1 sector into a
single data image artifact. Reassembling reconstructs the sector headers,
EDC and ECC from the user data.

Only MODE1/2352 sectors are supported. Rebuilding generates canonical sectors;
noncanonical sector bytes are not preserved.

The 'file_system' argument optionally extracts files from the decoded track.

Arguments (JSON):
  start_lba (default: 0)
    Unsigned integer, logical block address of the first sector in the BIN.
    Sector headers use MSF = LBA + 150 frames.
  file_system (default: raw)
    Available:
      - raw:     Produces a flat 2048-byte/sector data image
      - iso9660: Extract files using std/cdrom/iso9660

Presets:
  iso9660: {"file_system": "iso9660"}

Example:
  {"start_lba": 0, "file_system": "iso9660"}
"#;

const RAW_SECTOR_SIZE: usize = 2352;
const USER_SECTOR_SIZE: usize = 2048;
const LEAD_IN_FRAMES: u32 = 150;

pub(crate) struct Mode1;

#[derive(Debug, Deserialize)]
pub struct Mode1CodecArgs {
  #[serde(default)]
  start_lba: u32,
  #[serde(default)]
  file_system: CdRomFileSystem,
}

impl LudaredCodecArgs for Mode1CodecArgs {}

fn encode_sector(
  frame: u32,
  data: &[u8],
  forward: &[u8; 256],
  backward: &[u8; 256],
) -> Result<[u8; RAW_SECTOR_SIZE], String> {
  let mut sector = [0u8; RAW_SECTOR_SIZE];
  sector[..12].copy_from_slice(&CDROM_SECTOR_MAGIC);
  sector[12..15].copy_from_slice(&msf(frame)?);
  sector[15] = MODE1;

  // User data.
  sector[16..2064].copy_from_slice(data);

  // Error Detection Code.
  let crc = edc(&sector[..2064]);
  sector[2064..2068].copy_from_slice(&crc.to_le_bytes());

  // Reserved bytes 2068..2076 remain zero.

  // Error Correction Code P.
  let p = ecc_block(&sector[12..2076], 86, 24, 2, 86, forward, backward);

  sector[2076..2248].copy_from_slice(&p);

  // Error Correction Code Q.
  let q = ecc_block(&sector[12..2248], 52, 43, 86, 88, forward, backward);

  sector[2248..2352].copy_from_slice(&q);

  Ok(sector)
}

impl Codec for Mode1 {
  fn id(&self) -> &str {
    CODEC_ID
  }
  fn name(&self) -> &str {
    CODEC_NAME
  }
  fn description(&self) -> &str {
    CODEC_DESC
  }

  fn presets(&self) -> Vec<&str> {
    vec!["iso9660"]
  }

  fn preset_args(&self, preset: &str, args: Option<&str>) -> Result<String, CodecError> {
    let preset = match preset {
      "iso9660" => r#"{"file_system":"iso9660"}"#,
      _ => {
        return Err(CodecError::UndefinedPreset(
          preset.to_owned(),
          self.id().to_owned(),
        ));
      }
    };
    Ok(Mode1CodecArgs::merge(preset, args)?)
  }

  fn kind(&self) -> CodecKind {
    CodecKind::Rebuild
  }

  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    if data.is_empty() || !data.len().is_multiple_of(RAW_SECTOR_SIZE) {
      return Ok(CodecHandlingConfidence::No);
    }

    for sector in data.as_chunks::<RAW_SECTOR_SIZE>().0 {
      if sector[..12] != CDROM_SECTOR_MAGIC || sector[15] != MODE1 {
        return Ok(CodecHandlingConfidence::No);
      }
    }

    Ok(CodecHandlingConfidence::Certain)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("mode1".to_owned())
  }

  /// Extracts the 2048-byte user data from each raw MODE1/2352 sector.
  ///
  /// Validates sector boundaries, synchronization patterns and mode bytes.
  /// Sector headers, EDC and ECC are discarded and reconstructed during encoding.
  ///
  /// Returns a single `data.bin` artifact.
  /// `start_lba` is accepted but does not affect extraction.
  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    // Validate arguments, even though start_lba is only used when encoding.
    let mode1_args = Mode1CodecArgs::from_args(args)?;

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

      if sector[15] != MODE1 {
        return Err(CodecError::Message(format!(
          "Expected MODE1 sector at {index}, found mode {}",
          sector[15]
        )));
      }

      output.extend_from_slice(&sector[16..2064]);
    }

    match mode1_args.file_system {
      CdRomFileSystem::Raw => Ok(vec![DecodedArtifact {
        name: "data.bin".to_owned(),
        data: output,
      }]),
      CdRomFileSystem::Iso9660 => Iso9660.decode(&output, args),
    }
  }

  /// Rebuilds raw MODE1/2352 sectors from the `data.bin` artifact.
  ///
  /// Requires exactly one artifact containing a multiple of 2048 bytes.
  /// Generates sector headers, EDC and ECC for each sector.
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
    let mode1_args = Mode1CodecArgs::from_args(args)?;
    let artifacts = match mode1_args.file_system {
      CdRomFileSystem::Raw => artifacts,
      CdRomFileSystem::Iso9660 => &[DecodedArtifact {
        name: "data.bin".to_owned(),
        data: Iso9660.encode(vec![], artifacts, args)?,
      }],
    };

    if artifacts.len() != 1 || artifacts[0].name != "data.bin" {
      return Err(CodecError::Message(
        "Expected exactly one artifact named 'data.bin'".into(),
      ));
    }

    let data = &artifacts[0].data;
    if !data.len().is_multiple_of(USER_SECTOR_SIZE) {
      return Err(CodecError::Message(
        "Input size is not a multiple of 2048".into(),
      ));
    }

    let (forward, backward) = ecc_tables();
    let mut output = Vec::with_capacity(data.len() / USER_SECTOR_SIZE * RAW_SECTOR_SIZE);

    for (sector_index, chunk) in data.as_chunks::<USER_SECTOR_SIZE>().0.iter().enumerate() {
      let sector_index =
        u32::try_from(sector_index).map_err(|_| CodecError::Message("Too many sectors".into()))?;

      let frame = mode1_args
        .start_lba
        .checked_add(sector_index)
        .and_then(|lba| lba.checked_add(LEAD_IN_FRAMES))
        .ok_or_else(|| CodecError::Message("Sector address overflow".into()))?;

      let sector = encode_sector(frame, chunk, &forward, &backward).map_err(CodecError::Message)?;
      output.extend_from_slice(&sector);
    }

    Ok(output)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn detects_mode1_sectors() {
    let codec = Mode1;
    let (forward, backward) = ecc_tables();

    let sector = encode_sector(
      LEAD_IN_FRAMES,
      &[0u8; USER_SECTOR_SIZE],
      &forward,
      &backward,
    )
    .unwrap();

    assert_eq!(
      codec.can_handle(&sector).unwrap(),
      CodecHandlingConfidence::Certain,
    );

    assert_eq!(
      codec.can_handle(&sector[..RAW_SECTOR_SIZE - 1]).unwrap(),
      CodecHandlingConfidence::No,
    );

    let mut invalid_sync = sector;
    invalid_sync[1] = 0;

    assert_eq!(
      codec.can_handle(&invalid_sync).unwrap(),
      CodecHandlingConfidence::No,
    );

    let mut invalid_mode = sector;
    invalid_mode[15] = 2;

    assert_eq!(
      codec.can_handle(&invalid_mode).unwrap(),
      CodecHandlingConfidence::No,
    );

    assert_eq!(codec.can_handle(&[]).unwrap(), CodecHandlingConfidence::No,);
  }

  #[test]
  fn roundtrip_multiple_sectors() {
    let codec = Mode1;

    let mut data = vec![0u8; USER_SECTOR_SIZE * 4];

    data[USER_SECTOR_SIZE..USER_SECTOR_SIZE * 2].fill(0xFF);

    for (index, byte) in data[USER_SECTOR_SIZE * 2..].iter_mut().enumerate() {
      *byte = (index as u8).wrapping_mul(37).wrapping_add(13);
    }

    let artifacts = vec![DecodedArtifact {
      name: "data.bin".to_owned(),
      data: data.clone(),
    }];

    let encoded = codec.encode(Vec::new(), &artifacts, None).unwrap();
    let decoded = codec.decode(&encoded, None).unwrap();

    assert_eq!(encoded.len(), RAW_SECTOR_SIZE * 4);
    // Pinned before table-driven EDC: covers headers, payload, EDC and both ECC blocks.
    assert_eq!(
      crate::hash::sha256_bytes(&encoded),
      "fb1fbb777a57ad1c890c19b513589bc5708726734cb0eb3fc68f1a9ed1e6f7ad"
    );
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].data, data);
  }

  #[test]
  fn msf_addresses() {
    assert_eq!(msf(0).unwrap(), [0x00, 0x00, 0x00]);
    assert_eq!(msf(150).unwrap(), [0x00, 0x02, 0x00]);

    assert_eq!(msf(74).unwrap(), [0x00, 0x00, 0x74]);
    assert_eq!(msf(75).unwrap(), [0x00, 0x01, 0x00]);

    assert_eq!(msf(4499).unwrap(), [0x00, 0x59, 0x74]);
    assert_eq!(msf(4500).unwrap(), [0x01, 0x00, 0x00]);

    assert_eq!(
      msf(100 * 60 * 75),
      Err("MSF address exceeds 99:59:74".into()),
    );
  }

  #[test]
  fn edc_matches_bitwise_reference_at_chunk_boundaries() {
    let data: Vec<u8> = (0..2080)
      .map(|index| (index as u8).wrapping_mul(37).wrapping_add(13))
      .collect();

    for offset in 0..8 {
      for length in (0..=32).chain(2048..=2072) {
        let input = &data[offset..offset + length];
        let mut expected = 0u32;
        for &byte in input {
          expected ^= u32::from(byte);
          for _ in 0..8 {
            expected = (expected >> 1) ^ if expected & 1 != 0 { 0xD801_8001 } else { 0 };
          }
        }
        assert_eq!(edc(input), expected, "offset {offset}, length {length}");
      }
    }
  }

  #[test]
  fn edc_known_vector() {
    assert_eq!(edc(b""), 0x0000_0000);
    assert_eq!(edc(b"123456789"), 0x6EC2_EDC4);
  }
  #[test]
  fn encodes_nonzero_start_lba() {
    let codec = Mode1;

    let artifacts = vec![DecodedArtifact {
      name: "data.bin".to_owned(),
      data: vec![0u8; USER_SECTOR_SIZE * 2],
    }];

    let encoded = codec
      .encode(Vec::new(), &artifacts, Some(r#"{"start_lba":4500}"#))
      .unwrap();

    assert_eq!(&encoded[12..15], &[0x01, 0x02, 0x00]);
    assert_eq!(
      &encoded[RAW_SECTOR_SIZE + 12..RAW_SECTOR_SIZE + 15],
      &[0x01, 0x02, 0x01],
    );

    let decoded = codec.decode(&encoded, None).unwrap();

    assert_eq!(decoded[0].data, artifacts[0].data);
  }
}
