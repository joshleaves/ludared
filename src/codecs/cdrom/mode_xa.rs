use std::collections::HashMap;

use serde::Deserialize;

use super::CdRomFileSystem;
use super::common_mode::CDROM_SECTOR_MAGIC;
use super::common_mode::MODE2;
use super::common_mode::looks_like_xa;
use super::common_mode::msf;
use super::edc_ecc::ecc_block;
use super::edc_ecc::ecc_tables;
use super::edc_ecc::edc;
use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;

mod iso9660;
mod raw;

const CODEC_ID: &str = "std/cdrom/mode_xa";
const CODEC_NAME: &str = "CD-ROM Mode 2 XA (2352-byte sectors)";
const CODEC_DESC: &str = r#"Raw CD-ROM Mode 2 XA sector converter

Extracts a 2048-byte-per-sector ISO-compatible stream into data.bin.
For Form 2 sectors, data.bin retains the first 2048 payload bytes and
xa_data.bin stores the remaining 276 bytes. The four unique XA subheader bytes
per sector are stored in xa_subheaders.bin. The submode bit identifies Form 2
sectors whose additional payload bytes are stored in xa_data.bin.

Rebuilding repeats each subheader, regenerates canonical sector headers,
and recalculates EDC and (for Form 1) ECC. Form 2 sectors with omitted EDC
are not supported for lossless reconstruction.

Arguments (JSON):
  start_lba (default: 0)
    Logical block address of the first sector. MSF = LBA + 150 frames.
  file_system (default: raw)
    Available:
      - raw:     Produces a flat 2048-byte/sector data image
      - iso9660: Extract files using std/cdrom/iso9660

Presets:
  iso9660: {"file_system": "iso9660"}

Example:
  {"start_lba": 0, "file_system": "iso9660"}}
"#;

const RAW_SECTOR_SIZE: usize = 2352;
const FORM1_USER_SIZE: usize = 2048;
const FORM2_USER_SIZE: usize = 2324;
const SUBHEADER_SIZE: usize = 4;
const LEAD_IN_FRAMES: u32 = 150;

pub(crate) struct ModeXa;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeXaCodecArgs {
  #[serde(default)]
  pub start_lba: u32,
  #[serde(default)]
  file_system: CdRomFileSystem,
}

impl LudaredCodecArgs for ModeXaCodecArgs {}

fn is_form2(subheader: &[u8]) -> bool {
  subheader[2] & 0x20 != 0
}

fn encode_sector(
  frame: u32,
  subheader: &[u8],
  data: &[u8],
  forward: &[u8; 256],
  backward: &[u8; 256],
) -> Result<[u8; RAW_SECTOR_SIZE], String> {
  let mut sector = [0u8; RAW_SECTOR_SIZE];
  sector[..12].copy_from_slice(&CDROM_SECTOR_MAGIC);
  sector[12..15].copy_from_slice(&msf(frame)?);
  sector[15] = MODE2;
  sector[16..20].copy_from_slice(subheader);
  sector[20..24].copy_from_slice(subheader);

  let size = if is_form2(subheader) {
    FORM2_USER_SIZE
  } else {
    FORM1_USER_SIZE
  };
  sector[24..24 + size].copy_from_slice(data);

  if is_form2(subheader) {
    let crc = edc(&sector[16..2348]);
    sector[2348..2352].copy_from_slice(&crc.to_le_bytes());
  } else {
    let crc = edc(&sector[16..2072]);
    sector[2072..2076].copy_from_slice(&crc.to_le_bytes());

    // XA Form 1 ECC uses a zeroed address/mode field during calculation.
    let mut ecc_source = sector;
    ecc_source[12..16].fill(0);

    let p = ecc_block(&ecc_source[12..2076], 86, 24, 2, 86, forward, backward);
    sector[2076..2248].copy_from_slice(&p);
    ecc_source[2076..2248].copy_from_slice(&p);

    let q = ecc_block(&ecc_source[12..2248], 52, 43, 86, 88, forward, backward);
    sector[2248..2352].copy_from_slice(&q);
  }

  Ok(sector)
}

impl Codec for ModeXa {
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
    Ok(ModeXaCodecArgs::merge(preset, args)?)
  }

  fn kind(&self) -> CodecKind {
    CodecKind::Rebuild
  }

  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    if data.is_empty() || !data.len().is_multiple_of(RAW_SECTOR_SIZE) {
      return Ok(CodecHandlingConfidence::No);
    }

    for sector in data.as_chunks::<RAW_SECTOR_SIZE>().0 {
      if !sector.starts_with(&CDROM_SECTOR_MAGIC) || sector[15] != MODE2 || !looks_like_xa(sector) {
        return Ok(CodecHandlingConfidence::No);
      }
    }

    Ok(CodecHandlingConfidence::Certain)
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("mode_xa".to_owned())
  }

  /// Extracts a fixed-width 2048-byte stream and separate Form 2 payloads.
  ///
  /// Form 2 positions in data.bin are zero-filled placeholders. The original
  /// Form 2 data is kept in xa_data.bin; subheaders.bin stores four bytes per
  /// sector. EDC/ECC are regenerated on encoding.
  /// `start_lba` is accepted but does not affect extraction.
  #[cfg_attr(
    feature = "profiling",
    tracing::instrument(skip_all, fields(codec_id = %self.id(), operation = %"decode"))
  )]
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    let mode_xa_args = ModeXaCodecArgs::from_args(args)?;

    if !data.len().is_multiple_of(RAW_SECTOR_SIZE) {
      return Err(CodecError::Message(
        "Input size is not a multiple of 2352".into(),
      ));
    }

    let sector_count = data.len() / RAW_SECTOR_SIZE;
    let mut user_data = Vec::with_capacity(sector_count * FORM1_USER_SIZE);
    let mut xa_data = HashMap::<usize, [u8; 276]>::new();
    // let mut xa_data = Vec::new();
    let mut xa_subheaders = Vec::with_capacity(sector_count * SUBHEADER_SIZE);

    for (index, sector) in data.as_chunks::<RAW_SECTOR_SIZE>().0.iter().enumerate() {
      if !sector.starts_with(&CDROM_SECTOR_MAGIC) {
        return Err(CodecError::Message(format!(
          "Invalid sync pattern in sector {index}"
        )));
      }

      if sector[15] != MODE2 {
        return Err(CodecError::Message(format!(
          "Expected MODE2 sector at {index}, found mode {}",
          sector[15]
        )));
      }

      if sector[16..20] != sector[20..24] {
        return Err(CodecError::Message(format!(
          "Invalid XA subheader duplication in sector {index}"
        )));
      }

      if !looks_like_xa(sector) {
        return Err(CodecError::Message(format!(
          "Invalid or omitted XA EDC in sector {index}"
        )));
      }

      let subheader = &sector[16..20];
      xa_subheaders.extend_from_slice(subheader);

      user_data.extend_from_slice(&sector[24..24 + FORM1_USER_SIZE]);

      if is_form2(subheader) {
        xa_data.insert(
          index,
          sector[24 + FORM1_USER_SIZE..24 + FORM2_USER_SIZE]
            .try_into()
            .map_err(|_| CodecError::Message("Unreachable: Invalid XA data size".into()))?,
        );
      }
    }

    match mode_xa_args.file_system {
      CdRomFileSystem::Raw => raw::decode_xa_raw(user_data, xa_data, xa_subheaders),
      CdRomFileSystem::Iso9660 => iso9660::decode_xa_iso9660(user_data, xa_data, xa_subheaders),
    }
  }

  /// Rebuilds XA sectors from fixed-width user data, Form 2 data and subheaders.
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
    let mode_xa_args = ModeXaCodecArgs::from_args(args)?;

    match mode_xa_args.file_system {
      CdRomFileSystem::Raw => raw::encode_xa_raw(artifacts, mode_xa_args),
      CdRomFileSystem::Iso9660 => iso9660::encode_xa_iso9660(artifacts, mode_xa_args),
    }
  }
}
