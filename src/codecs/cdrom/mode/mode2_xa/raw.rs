use super::*;

const XA_ENTRY_SIZE: usize = 4 + (FORM2_USER_SIZE - FORM1_USER_SIZE);

// CD-ROM Mode XA: Raw mode decode
//
// Produces three artifacts:
//
// - data.bin:
//   Concatenated first 2048 user-data bytes of every sector (Form 1 and Form 2).
//
// - xa_data.bin:
//   One 280-byte entry per Form 2 sector, sorted by sector index:
//     [sector_index: u32 LE][extra_data: u8; 276]
//   Sector indices are zero-based, relative to the beginning of the track.
//
// - xa_subheaders.bin:
//   One 4-byte entry per sector, in physical sector order:
//     [file_number: u8][channel_number: u8][submode: u8][coding_info: u8]
//   Includes both Form 1 and Form 2 sectors. The duplicated XA subheader
//   is not stored, as it can be regenerated during encoding.
pub(super) fn decode_xa_raw(
  user_data: Vec<u8>,
  xa_data: HashMap<usize, [u8; 276]>,
  xa_subheaders: Vec<u8>,
) -> Result<Vec<DecodedArtifact>, CodecError> {
  let mut entries: Vec<_> = xa_data.iter().collect();
  entries.sort_unstable_by_key(|(index, _)| *index);

  let mut xa_data_serialized = Vec::with_capacity(entries.len() * XA_ENTRY_SIZE);

  for (&index, data) in entries {
    let index = u32::try_from(index)
      .map_err(|_| CodecError::Message("XA sector index exceeds u32".into()))?;

    xa_data_serialized.extend_from_slice(&index.to_le_bytes());
    xa_data_serialized.extend_from_slice(data);
  }

  Ok(vec![
    DecodedArtifact {
      name: "data.bin".to_owned(),
      data: user_data,
    },
    DecodedArtifact {
      name: "xa_data.bin".to_owned(),
      data: xa_data_serialized,
    },
    DecodedArtifact {
      name: "xa_subheaders.bin".to_owned(),
      data: xa_subheaders,
    },
  ])
}

/// Rebuilds a raw Mode 2 XA track from its three decoded artifacts:
///
/// - `data.bin`: 2048 user-data bytes per sector (Form 1 and Form 2).
/// - `xa_data.bin`: 280-byte entries containing a sector index (u32 LE)
///   followed by 276 additional Form 2 payload bytes.
/// - `xa_subheaders.bin`: 4-byte XA subheaders for every sector.
///
/// Reconstructs 2352-byte sectors, regenerating headers, duplicated XA
/// subheaders, EDC and ECC. Sector addresses are derived from `start_lba`.
pub(super) fn encode_xa_raw(
  artifacts: &[DecodedArtifact],
  mode_xa_args: Mode2XaCodecArgs,
) -> Result<Vec<u8>, CodecError> {
  if artifacts.len() != 3
    || artifacts[0].name != "data.bin"
    || artifacts[1].name != "xa_data.bin"
    || artifacts[2].name != "xa_subheaders.bin"
  {
    return Err(CodecError::Message(
      "Expected 'data.bin', 'xa_data.bin' and 'xa_subheaders.bin' artifacts, in that order".into(),
    ));
  }

  let data = &artifacts[0].data;
  let xa_data = &artifacts[1].data;
  let subheaders = &artifacts[2].data;

  if !subheaders.len().is_multiple_of(SUBHEADER_SIZE) {
    return Err(CodecError::Message(
      "XA subheaders size is not a multiple of 4".into(),
    ));
  }

  if !xa_data.len().is_multiple_of(XA_ENTRY_SIZE) {
    return Err(CodecError::Message(
      "XA data size is not a multiple of 280".into(),
    ));
  }

  let sector_count = subheaders.len() / SUBHEADER_SIZE;
  let expected_data_size = sector_count * FORM1_USER_SIZE;

  if data.len() != expected_data_size {
    return Err(CodecError::Message(format!(
      "Expected {expected_data_size} data bytes, found {}",
      data.len()
    )));
  }

  let form2_count = subheaders
    .as_chunks::<SUBHEADER_SIZE>()
    .0
    .iter()
    .filter(|subheader| is_form2(*subheader))
    .count();

  if xa_data.len() / XA_ENTRY_SIZE != form2_count {
    return Err(CodecError::Message(format!(
      "Expected {form2_count} XA entries, found {}",
      xa_data.len() / XA_ENTRY_SIZE
    )));
  }

  let (forward, backward) = ecc_tables();
  let mut output = Vec::with_capacity(sector_count * RAW_SECTOR_SIZE);
  let mut xa_offset = 0;

  for (index, subheader) in subheaders
    .as_chunks::<SUBHEADER_SIZE>()
    .0
    .iter()
    .enumerate()
  {
    let sector_index =
      u32::try_from(index).map_err(|_| CodecError::Message("Too many sectors".into()))?;

    let frame = mode_xa_args
      .start_lba
      .checked_add(sector_index)
      .and_then(|lba| lba.checked_add(LEAD_IN_FRAMES))
      .ok_or_else(|| CodecError::Message("Sector address overflow".into()))?;

    let data_offset = index * FORM1_USER_SIZE;
    let data_chunk = &data[data_offset..data_offset + FORM1_USER_SIZE];

    let sector = if is_form2(subheader) {
      let entry = &xa_data[xa_offset..xa_offset + XA_ENTRY_SIZE];

      let entry_index = u32::from_le_bytes(
        entry[..4]
          .try_into()
          .map_err(|_| CodecError::Message("Invalid XA sector index".into()))?,
      );

      if entry_index != index as u32 {
        return Err(CodecError::Message(format!(
          "Expected XA entry for sector {index}, found sector {entry_index}"
        )));
      }

      let mut payload = [0u8; FORM2_USER_SIZE];

      payload[..FORM1_USER_SIZE].copy_from_slice(data_chunk);
      payload[FORM1_USER_SIZE..].copy_from_slice(&entry[4..]);

      xa_offset += XA_ENTRY_SIZE;

      encode_sector(frame, subheader, &payload, &forward, &backward)
    } else {
      encode_sector(frame, subheader, data_chunk, &forward, &backward)
    }
    .map_err(CodecError::Message)?;

    output.extend_from_slice(&sector);
  }

  Ok(output)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn roundtrip_mixed_forms() {
    let codec = Mode2Xa;

    // Form 1, Form 2, Form 1.
    let subheaders = vec![
      1,
      0,
      0x08,
      0,
      1,
      1,
      0x24,
      0,
      2,
      0,
      0x88,
      0,
    ];

    let mut data = vec![0u8; FORM1_USER_SIZE * 3];

    for (index, byte) in data.iter_mut().enumerate() {
      *byte = (index as u8).wrapping_mul(37).wrapping_add(13);
    }

    // XA entry: sector index (u32 LE) + 276 additional bytes.
    let mut xa_data = Vec::new();
    xa_data.extend_from_slice(&1u32.to_le_bytes());

    for index in 0..(FORM2_USER_SIZE - FORM1_USER_SIZE) {
      xa_data.push((index as u8).wrapping_mul(11).wrapping_add(3));
    }

    let artifacts = vec![
      DecodedArtifact {
        name: "data.bin".to_owned(),
        data: data.clone(),
      },
      DecodedArtifact {
        name: "xa_data.bin".to_owned(),
        data: xa_data.clone(),
      },
      DecodedArtifact {
        name: "xa_subheaders.bin".to_owned(),
        data: subheaders.clone(),
      },
    ];

    let encoded = codec.encode(Vec::new(), &artifacts, None).unwrap();

    assert_eq!(encoded.len(), RAW_SECTOR_SIZE * 3);
    assert_eq!(
      codec.can_handle(&encoded).unwrap(),
      CodecHandlingConfidence::Certain
    );

    let decoded = codec.decode(&encoded, None).unwrap();

    assert_eq!(decoded.len(), 3);
    assert_eq!(decoded[0].data, data);
    assert_eq!(decoded[1].data, xa_data);
    assert_eq!(decoded[2].data, subheaders);

    assert_eq!(codec.encode(Vec::new(), &decoded, None).unwrap(), encoded);
  }

  #[test]
  fn rejects_invalid_form2_sector_index() {
    let codec = Mode2Xa;

    let mut xa_data = Vec::new();

    // Incorrect index: expected sector 0, got sector 1.
    xa_data.extend_from_slice(&1u32.to_le_bytes());
    xa_data.extend_from_slice(&[0u8; FORM2_USER_SIZE - FORM1_USER_SIZE]);

    let artifacts = vec![
      DecodedArtifact {
        name: "data.bin".to_owned(),
        data: vec![1; FORM1_USER_SIZE],
      },
      DecodedArtifact {
        name: "xa_data.bin".to_owned(),
        data: xa_data,
      },
      DecodedArtifact {
        name: "xa_subheaders.bin".to_owned(),
        data: vec![1, 0, 0x20, 0],
      },
    ];

    assert!(codec.encode(Vec::new(), &artifacts, None).is_err());
  }

  #[test]
  fn rejects_missing_form2_entry() {
    let codec = Mode2Xa;

    let artifacts = vec![
      DecodedArtifact {
        name: "data.bin".to_owned(),
        data: vec![0; FORM1_USER_SIZE],
      },
      DecodedArtifact {
        name: "xa_data.bin".to_owned(),
        data: Vec::new(),
      },
      DecodedArtifact {
        name: "xa_subheaders.bin".to_owned(),
        data: vec![1, 0, 0x20, 0],
      },
    ];

    assert!(codec.encode(Vec::new(), &artifacts, None).is_err());
  }

  #[test]
  fn rejects_mismatched_artifact_lengths() {
    let codec = Mode2Xa;

    let artifacts = vec![
      DecodedArtifact {
        name: "data.bin".to_owned(),
        data: vec![0; FORM1_USER_SIZE - 1],
      },
      DecodedArtifact {
        name: "xa_data.bin".to_owned(),
        data: Vec::new(),
      },
      DecodedArtifact {
        name: "xa_subheaders.bin".to_owned(),
        data: vec![1, 0, 0x08, 0],
      },
    ];

    assert!(codec.encode(Vec::new(), &artifacts, None).is_err());
  }
}
