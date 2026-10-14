use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use serde::Deserialize;

const CODEC_ID: &str = "std/cdrom/mode2352";
const CODEC_NAME: &str = "CD-ROM Mode 1 (2352-byte sectors)";
const CODEC_DESC: &str = r#"Raw CD-ROM Mode 1 sector converter

Extracts the 2048-byte user data from each 2352-byte MODE1 sector into a
single ISO image artifact. Reassembling reconstructs the sector headers,
EDC and ECC from the user data.

Only MODE1/2352 sectors are supported. Rebuilding generates canonical sectors;
noncanonical sector bytes are not preserved.

Arguments (JSON):
  start_lba (default: 0)
    Unsigned integer, logical block address of the first sector in the BIN.
    Sector headers use MSF = LBA + 150 frames.

Example:
  {"start_lba":0}
"#;

const RAW_SECTOR_SIZE: usize = 2352;
const USER_SECTOR_SIZE: usize = 2048;
const LEAD_IN_FRAMES: u32 = 150;

pub(crate) struct Mode2352;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mode2352CodecArgs {
  #[serde(default)]
  pub start_lba: u32,
}

impl LudaredCodecArgs for Mode2352CodecArgs {}

fn bcd(value: u32) -> u8 {
  ((value / 10) << 4 | (value % 10)) as u8
}

fn msf(frame: u32) -> Result<[u8; 3], String> {
  let minutes = frame / (75 * 60);

  if minutes > 99 {
    return Err("MSF address exceeds 99:59:74".into());
  }

  Ok([
    bcd(minutes),
    bcd((frame / 75) % 60),
    bcd(frame % 75),
  ])
}

fn edc(data: &[u8]) -> u32 {
  let mut crc = 0u32;

  for &byte in data {
    crc ^= byte as u32;

    for _ in 0..8 {
      crc = (crc >> 1) ^ if crc & 1 != 0 { 0xD801_8001 } else { 0 };
    }
  }

  crc
}

fn ecc_tables() -> ([u8; 256], [u8; 256]) {
  let mut forward = [0u8; 256];
  let mut backward = [0u8; 256];

  for i in 0..256 {
    let j = ((i << 1) ^ if i & 0x80 != 0 { 0x11D } else { 0 }) as u8;

    forward[i] = j;
    backward[(i as u8 ^ j) as usize] = i as u8;
  }

  (forward, backward)
}

fn ecc_block(
  source: &[u8],
  major_count: usize,
  minor_count: usize,
  major_mult: usize,
  minor_inc: usize,
  forward: &[u8; 256],
  backward: &[u8; 256],
) -> Vec<u8> {
  let size = major_count * minor_count;
  let mut result = vec![0u8; major_count * 2];

  for major in 0..major_count {
    let mut index = (major >> 1) * major_mult + (major & 1);
    let mut a = 0u8;
    let mut b = 0u8;

    for _ in 0..minor_count {
      let value = source[index];

      index += minor_inc;

      if index >= size {
        index -= size;
      }

      a ^= value;
      b ^= value;
      a = forward[a as usize];
    }

    a = backward[(forward[a as usize] ^ b) as usize];

    result[major] = a;
    result[major + major_count] = a ^ b;
  }

  result
}

fn encode_sector(
  frame: u32,
  data: &[u8],
  forward: &[u8; 256],
  backward: &[u8; 256],
) -> Result<[u8; RAW_SECTOR_SIZE], String> {
  let mut sector = [0u8; RAW_SECTOR_SIZE];

  // Sync pattern.
  sector[1..11].fill(0xFF);

  // Sector address and mode.
  sector[12..15].copy_from_slice(&msf(frame)?);
  sector[15] = 0x01;

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

impl Codec for Mode2352 {
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
    if data.is_empty() || !data.len().is_multiple_of(RAW_SECTOR_SIZE) {
      return CodecHandlingConfidence::No;
    }

    let sector = &data[..RAW_SECTOR_SIZE];

    if sector[0] != 0
      || !sector[1..11].iter().all(|&byte| byte == 0xFF)
      || sector[11] != 0
      || sector[15] != 1
    {
      return CodecHandlingConfidence::No;
    }

    CodecHandlingConfidence::Likely
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("mode2352".to_owned())
  }

  /// Extracts the 2048-byte user data from each raw MODE1/2352 sector.
  ///
  /// Validates sector boundaries, synchronization patterns and mode bytes.
  /// Sector headers, EDC and ECC are discarded and reconstructed during encoding.
  ///
  /// Returns a single `data.bin` artifact.
  /// `start_lba` is accepted but does not affect extraction.
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    // Validate arguments, even though start_lba is only used when encoding.
    let _args = Mode2352CodecArgs::from_args(args)?;

    if !data.len().is_multiple_of(RAW_SECTOR_SIZE) {
      return Err(CodecError::Message(
        "Input size is not a multiple of 2352".into(),
      ));
    }

    let mut output = Vec::with_capacity(data.len() / RAW_SECTOR_SIZE * USER_SECTOR_SIZE);

    for (index, sector) in data.as_chunks::<RAW_SECTOR_SIZE>().0.iter().enumerate() {
      if sector[0] != 0 || !sector[1..11].iter().all(|&byte| byte == 0xFF) || sector[11] != 0 {
        return Err(CodecError::Message(format!(
          "Invalid sync pattern in sector {index}"
        )));
      }

      if sector[15] != 1 {
        return Err(CodecError::Message(format!(
          "Expected MODE1 sector at {index}, found mode {}",
          sector[15]
        )));
      }

      output.extend_from_slice(&sector[16..2064]);
    }

    Ok(vec![DecodedArtifact {
      name: "data.bin".to_owned(),
      data: output,
    }])
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
  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    let args = Mode2352CodecArgs::from_args(args)?;

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

      let frame = args
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
    let codec = Mode2352;
    let (forward, backward) = ecc_tables();

    let sector = encode_sector(
      LEAD_IN_FRAMES,
      &[0u8; USER_SECTOR_SIZE],
      &forward,
      &backward,
    )
    .unwrap();

    assert_eq!(codec.can_handle(&sector), CodecHandlingConfidence::Likely,);

    assert_eq!(
      codec.can_handle(&sector[..RAW_SECTOR_SIZE - 1]),
      CodecHandlingConfidence::No,
    );

    let mut invalid_sync = sector;
    invalid_sync[1] = 0;

    assert_eq!(codec.can_handle(&invalid_sync), CodecHandlingConfidence::No,);

    let mut invalid_mode = sector;
    invalid_mode[15] = 2;

    assert_eq!(codec.can_handle(&invalid_mode), CodecHandlingConfidence::No,);

    assert_eq!(codec.can_handle(&[]), CodecHandlingConfidence::No,);
  }

  #[test]
  fn roundtrip_multiple_sectors() {
    let codec = Mode2352;

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
  fn rejects_invalid_inputs() {
    let codec = Mode2352;

    assert!(codec.decode(&[0u8; 2351], None).is_err());

    let invalid_artifact = |data: Vec<u8>| {
      vec![DecodedArtifact {
        name: "data.bin".to_owned(),
        data,
      }]
    };

    assert!(
      codec
        .encode(Vec::new(), &invalid_artifact(vec![0u8; 2047]), None)
        .is_err()
    );

    assert!(
      codec
        .encode(
          Vec::new(),
          &invalid_artifact(vec![0u8; USER_SECTOR_SIZE]),
          Some(r#"{"start_lba":-1}"#),
        )
        .is_err()
    );

    assert!(
      codec
        .encode(
          Vec::new(),
          &invalid_artifact(vec![0u8; USER_SECTOR_SIZE]),
          Some(r#"{"unknown":true}"#),
        )
        .is_err()
    );

    assert!(codec.encode(Vec::new(), &[], None).is_err());
  }

  #[test]
  fn edc_known_vector() {
    assert_eq!(edc(b""), 0x0000_0000);
    assert_eq!(edc(b"123456789"), 0x6EC2_EDC4);
  }
  #[test]
  fn encodes_nonzero_start_lba() {
    let codec = Mode2352;

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

  #[test]
  #[ignore = "Requires a local Sakura Taisen BIN image"]
  fn sakura_taisen_track1_roundtrip() {
    let path = "/Users/red/Games/Sega Saturn/#ISOs/Sakura Taisen (Japan) (Disc 1) (6M)/Sakura Taisen (Japan) (Disc 1) (6M) (Track 1).bin";

    let original = std::fs::read(path).expect("Failed to read Sakura Taisen BIN");
    let codec = Mode2352;

    assert_eq!(codec.can_handle(&original), CodecHandlingConfidence::Likely,);

    let artifacts = codec.decode(&original, None).unwrap();

    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].name, "data.bin");
    assert_eq!(
      artifacts[0].data.len(),
      original.len() / RAW_SECTOR_SIZE * USER_SECTOR_SIZE,
    );

    let rebuilt = codec.encode(Vec::new(), &artifacts, None).unwrap();

    assert_eq!(rebuilt, original, "Roundtrip mismatch");
  }
}
