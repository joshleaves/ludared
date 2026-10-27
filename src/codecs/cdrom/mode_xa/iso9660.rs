use super::*;
use crate::codecs::cdrom::iso9660::Iso9660;
use crate::codecs::cdrom::iso9660::metadata::IsoMetadata;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

const EXTRA: usize = FORM2_USER_SIZE - FORM1_USER_SIZE;
const XA_METADATA: &str = "XA.json";

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct XaMetadata {
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  file_number: BTreeMap<String, u8>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  sector_overrides: BTreeMap<usize, String>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  orphan_form2_data: BTreeMap<usize, String>,
}

fn err(message: impl Into<String>) -> CodecError {
  CodecError::Message(message.into())
}

fn hex(bytes: &[u8]) -> String {
  const DIGITS: &[u8; 16] = b"0123456789abcdef";
  let mut result = String::with_capacity(bytes.len() * 2);
  for byte in bytes {
    result.push(DIGITS[(byte >> 4) as usize] as char);
    result.push(DIGITS[(byte & 15) as usize] as char);
  }
  result
}

fn unhex(value: &str, expected: usize) -> Result<Vec<u8>, CodecError> {
  if value.len() != expected * 2 {
    return Err(err(format!(
      "Expected {} hex characters, got {}",
      expected * 2,
      value.len()
    )));
  }
  (0..value.len())
    .step_by(2)
    .map(|i| {
      u8::from_str_radix(&value[i..i + 2], 16)
        .map_err(|_| err(format!("Invalid hex string: {value}")))
    })
    .collect()
}

fn sectors(length: usize) -> usize {
  length.div_ceil(FORM1_USER_SIZE)
}

// Canonical mastering convention observed in Fuuraiki.
fn default_file_submode(last: bool) -> u8 {
  if last { 0x89 } else { 0x08 }
}

fn default_iso_submode(index: usize, metadata: &IsoMetadata) -> u8 {
  if index == 16 {
    return 0x09;
  }
  if index == 17 {
    return 0x89;
  }
  let table = &metadata.layout.path_table;
  let table_sectors = sectors(table.size());
  for start in [
    table.l_sector,
    table.m_sector,
    table.optional_l_sector,
    table.optional_m_sector,
  ] {
    if start != 0 && index >= start as usize && index < start as usize + table_sectors {
      return 0x89;
    }
  }
  for directory in &metadata.layout.directories {
    let start = directory.extent as usize;
    let count = sectors(directory.length as usize);
    if count != 0 && index == start + count - 1 {
      return 0x89;
    }
  }
  0x08
}

// A file's logical ISO extent always counts 2048-byte blocks, including Form 2.
fn file_ranges(
  metadata: &IsoMetadata,
  image_sectors: usize,
) -> Result<Vec<(String, usize, usize)>, CodecError> {
  let mut ranges = Vec::new();
  let mut used = HashSet::new();
  for directory in &metadata.layout.directories {
    for record in &directory.records {
      if !record.is_file() {
        continue;
      }
      let name = if directory.path.is_empty() {
        record.artifact_name().to_owned()
      } else {
        format!("{}/{}", directory.path, record.artifact_name())
      };
      if !used.insert(name.clone()) {
        return Err(err(format!("Duplicate ISO file: {name}")));
      }
      let start = record.extent as usize;
      let count = sectors(record.length as usize);
      if start >= image_sectors {
        continue;
      } // Out-of-bounds file: ISO codec preserves its record.
      if start
        .checked_add(count)
        .is_none_or(|end| end > image_sectors)
      {
        return Err(err(format!("File {name} crosses the image boundary")));
      }
      ranges.push((name, start, count));
    }
  }
  let mut occupied = HashSet::new();
  for (name, start, count) in &ranges {
    for index in *start..*start + *count {
      if !occupied.insert(index) {
        return Err(err(format!(
          "Overlapping ISO file extent: {name}, sector {index}"
        )));
      }
    }
  }
  Ok(ranges)
}

pub(super) fn decode_xa_iso9660(
  user_data: Vec<u8>,
  xa_data: HashMap<usize, [u8; EXTRA]>,
  xa_subheaders: Vec<u8>,
) -> Result<Vec<DecodedArtifact>, CodecError> {
  if !user_data.len().is_multiple_of(FORM1_USER_SIZE)
    || xa_subheaders.len() != user_data.len() / FORM1_USER_SIZE * SUBHEADER_SIZE
  {
    return Err(err("XA data/subheader length mismatch"));
  }
  let count = user_data.len() / FORM1_USER_SIZE;
  let mut iso = Iso9660.decode(&user_data, None)?;
  let metadata =
    IsoMetadata::from_json(&iso.first().ok_or_else(|| err("Missing ISO metadata"))?.data)?;
  let ranges = file_ranges(&metadata, count)?;
  let mut xa = XaMetadata::default();
  let mut covered = vec![false; count];
  let mut names = HashSet::new();

  for (name, start, n) in &ranges {
    let file = iso
      .iter_mut()
      .find(|artifact| artifact.name == *name)
      .ok_or_else(|| err(format!("ISO file artifact missing: {name}")))?;
    names.insert(name.clone());
    let mut sidecar = Vec::with_capacity(n * 3);
    let mut noncanonical = false;
    let mut expanded = Vec::with_capacity(file.data.len() + n * EXTRA);
    let file_number = xa_subheaders[start * 4];
    for local in 0..*n {
      let index = start + local;
      covered[index] = true;
      let sub = &xa_subheaders[index * 4..index * 4 + 4];
      if sub[0] != file_number {
        return Err(err(format!(
          "File number varies inside {name} at sector {index}"
        )));
      }
      sidecar.extend_from_slice(&sub[1..4]);
      if sub[1] != 0 || sub[2] != default_file_submode(local + 1 == *n) || sub[3] != 0 {
        noncanonical = true;
      }
      let offset = local * FORM1_USER_SIZE;
      let end = (offset + FORM1_USER_SIZE).min(file.data.len());
      if offset < end {
        expanded.extend_from_slice(&file.data[offset..end]);
      }
      if is_form2(sub) {
        let extra = xa_data
          .get(&index)
          .ok_or_else(|| err(format!("Missing Form 2 data at {index}")))?;
        // Preserve the 276-byte extension after the full 2048-byte sector prefix.
        // ISO9660 files with a partial final Form 2 sector cannot be represented losslessly here.
        if end != offset + FORM1_USER_SIZE {
          return Err(err(format!(
            "Partial final Form 2 sector in {name} is unsupported"
          )));
        }
        expanded.extend_from_slice(extra);
      }
    }
    if file_number != 0 {
      xa.file_number.insert(name.clone(), file_number);
    }
    file.data = expanded;
    if noncanonical {
      iso.push(DecodedArtifact {
        name: format!("{name}.XA"),
        data: sidecar,
      });
    }
  }

  for index in 0..count {
    if covered[index] {
      continue;
    }
    let sub = &xa_subheaders[index * 4..index * 4 + 4];
    let default = [
      0,
      0,
      default_iso_submode(index, &metadata),
      0,
    ];
    if sub != default {
      xa.sector_overrides.insert(index, hex(sub));
    }
    if is_form2(sub) {
      let extra = xa_data
        .get(&index)
        .ok_or_else(|| err(format!("Missing orphan Form 2 data at {index}")))?;
      if extra.iter().any(|&byte| byte != 0) {
        xa.orphan_form2_data.insert(index, hex(extra));
      }
    }
  }
  let mut result = Vec::with_capacity(iso.len() + 1);
  result.push(DecodedArtifact {
    name: XA_METADATA.to_owned(),
    data: serde_json::to_vec_pretty(&xa).map_err(|e| err(e.to_string()))?,
  });
  result.extend(iso);
  Ok(result)
}

pub(super) fn encode_xa_iso9660(
  artifacts: &[DecodedArtifact],
  args: ModeXaCodecArgs,
) -> Result<Vec<u8>, CodecError> {
  let (xa_artifact, rest) = artifacts
    .split_first()
    .ok_or_else(|| err("Missing XA.json"))?;
  if xa_artifact.name != XA_METADATA {
    return Err(err("Expected XA.json as first artifact"));
  }
  let xa: XaMetadata = serde_json::from_slice(&xa_artifact.data).map_err(|e| err(e.to_string()))?;
  let iso_metadata_artifact = rest
    .iter()
    .find(|a| a.name == "ISO9660.json")
    .ok_or_else(|| err("Missing ISO9660.json"))?;
  let original_metadata = IsoMetadata::from_json(&iso_metadata_artifact.data)?;
  let mut logical_artifacts = Vec::new();
  logical_artifacts.push(DecodedArtifact {
    name: "ISO9660.json".into(),
    data: iso_metadata_artifact.data.clone(),
  });
  let mut extra_by_file: HashMap<String, Vec<(usize, [u8; EXTRA])>> = HashMap::new();
  let mut sub_by_file: HashMap<String, Vec<[u8; 3]>> = HashMap::new();
  let mut sidecar_names: HashSet<String> = rest
    .iter()
    .filter(|artifact| artifact.name.ends_with(".XA"))
    .map(|artifact| artifact.name.clone())
    .collect();
  for artifact in rest {
    if artifact.name == "ISO9660.json" {
      continue;
    }
    if artifact.name.ends_with(".XA") {
      continue;
    }
    let name = &artifact.name;
    let sidecar_name = format!("{name}.XA");
    let sidecar = rest.iter().find(|a| a.name == sidecar_name);
    if sidecar.is_some() {
      sidecar_names.remove(&sidecar_name);
    }
    let mut logical = Vec::new();
    let mut extras = Vec::new();
    let mut subs = Vec::new();
    if let Some(sidecar) = sidecar {
      if !sidecar.data.len().is_multiple_of(3) {
        return Err(err(format!("Invalid sidecar length: {sidecar_name}")));
      }
      let n = sidecar.data.len() / 3;
      let mut offset = 0;
      for local in 0..n {
        let triple: [u8; 3] = sidecar.data[local * 3..local * 3 + 3]
          .try_into()
          .map_err(|_| CodecError::Message("Unreachable: Length already verified".into()))?;
        let form2 = triple[1] & 0x20 != 0;
        let size = if form2 {
          FORM2_USER_SIZE
        } else {
          FORM1_USER_SIZE
        };
        let remaining = artifact.data.len().saturating_sub(offset);
        let take = remaining.min(size);
        if local + 1 != n && take != size {
          return Err(err(format!("Truncated XA payload: {name}")));
        }
        if form2 && take != size {
          return Err(err(format!("Partial Form 2 payload unsupported: {name}")));
        }
        let prefix = take.min(FORM1_USER_SIZE);
        logical.extend_from_slice(&artifact.data[offset..offset + prefix]);
        if form2 {
          let mut extra = [0; EXTRA];
          extra.copy_from_slice(&artifact.data[offset + FORM1_USER_SIZE..offset + FORM2_USER_SIZE]);
          extras.push((local, extra));
        }
        subs.push(triple);
        offset += take;
      }
      if offset != artifact.data.len() {
        return Err(err(format!(
          "XA sidecar sector count does not match {name}"
        )));
      }
    } else {
      logical.extend_from_slice(&artifact.data);
    }
    if !extras.is_empty() {
      extra_by_file.insert(name.clone(), extras);
    }
    if !subs.is_empty() {
      sub_by_file.insert(name.clone(), subs);
    }
    logical_artifacts.push(DecodedArtifact {
      name: name.clone(),
      data: logical,
    });
  }
  if !sidecar_names.is_empty() {
    return Err(err(format!("Unmatched XA sidecars: {sidecar_names:?}")));
  }

  // Use shift to preserve the relative order of physical regions when files grow.
  // The metadata is re-decoded after rebuilding to discover updated extents.
  let iso_args = r#"{"placement":"shift"}"#;
  let rebuilt = Iso9660.encode(Vec::new(), &logical_artifacts, Some(iso_args))?;
  if !rebuilt.len().is_multiple_of(FORM1_USER_SIZE) {
    return Err(err("Rebuilt ISO has partial sectors"));
  }
  let count = rebuilt.len() / FORM1_USER_SIZE;
  let new_iso = Iso9660.decode(&rebuilt, None)?;
  let new_metadata = IsoMetadata::from_json(&new_iso[0].data)?;
  let ranges = file_ranges(&new_metadata, count)?;
  let old_ranges = file_ranges(
    &original_metadata,
    original_metadata.layout.image_sectors as usize,
  )?;
  let mut old_by_name = HashMap::new();
  for (name, start, n) in old_ranges {
    old_by_name.insert(name, (start, n));
  }
  let mut headers = vec![[0u8, 0, 0x08, 0]; count];
  let mut form2_extras = HashMap::<usize, [u8; EXTRA]>::new();
  let mut covered = vec![false; count];
  for (index, header) in headers.iter_mut().enumerate() {
    header[2] = default_iso_submode(index, &new_metadata);
  }

  for (name, start, n) in &ranges {
    let number = xa.file_number.get(name).copied().unwrap_or(0);
    let subs = sub_by_file.get(name);
    if let Some(subs) = subs
      && subs.len() != *n
    {
      return Err(err(format!(
        "XA sidecar for {name} has {} sectors, rebuilt ISO has {n}",
        subs.len()
      )));
    }
    for local in 0..*n {
      let index = start + local;
      covered[index] = true;
      let triple = subs.map_or(
        [
          0,
          default_file_submode(local + 1 == *n),
          0,
        ],
        |v| v[local],
      );
      headers[index] = [
        number,
        triple[0],
        triple[1],
        triple[2],
      ];
    }
    if let Some(extras) = extra_by_file.get(name) {
      for (local, extra) in extras {
        form2_extras.insert(start + local, *extra);
      }
    }
  }

  // Sector overrides are anchored to physical locations. If ISO layout changed,
  // translating arbitrary non-file regions is ambiguous: refuse rather than corrupt.
  if (original_metadata.layout.image_sectors as usize != count
    || ranges
      .iter()
      .any(|(name, start, _)| old_by_name.get(name).is_some_and(|(old, _)| old != start)))
    && (!xa.sector_overrides.is_empty() || !xa.orphan_form2_data.is_empty())
  {
    return Err(err(
      "Cannot safely relocate absolute XA sector overrides after ISO layout changes",
    ));
  }
  for (index, value) in &xa.sector_overrides {
    if *index >= count || covered[*index] {
      return Err(err(format!(
        "Invalid/or overlapping sector override: {index}"
      )));
    }
    headers[*index].copy_from_slice(&unhex(value, 4)?);
  }
  for (index, value) in &xa.orphan_form2_data {
    if *index >= count || covered[*index] || !is_form2(&headers[*index]) {
      return Err(err(format!(
        "Invalid orphan Form 2 payload sector: {index}"
      )));
    }
    form2_extras.insert(
      *index,
      unhex(value, EXTRA)?
        .try_into()
        .map_err(|_| err("Unreachable: Form 2 payload length already verified"))?,
    );
  }
  let (forward, backward) = ecc_tables();
  let mut output = Vec::with_capacity(count * RAW_SECTOR_SIZE);
  for (index, sub) in headers.iter().enumerate() {
    let index_u32 = u32::try_from(index).map_err(|_| err("Too many sectors"))?;
    let frame = args
      .start_lba
      .checked_add(index_u32)
      .and_then(|n| n.checked_add(LEAD_IN_FRAMES))
      .ok_or_else(|| err("Sector address overflow"))?;
    let logical = &rebuilt[index * FORM1_USER_SIZE..(index + 1) * FORM1_USER_SIZE];
    let sector = if is_form2(sub) {
      let mut payload = [0u8; FORM2_USER_SIZE];
      payload[..FORM1_USER_SIZE].copy_from_slice(logical);
      if let Some(extra) = form2_extras.get(&index) {
        payload[FORM1_USER_SIZE..].copy_from_slice(extra);
      }
      encode_sector(frame, sub, &payload, &forward, &backward)
    } else {
      encode_sector(frame, sub, logical, &forward, &backward)
    }
    .map_err(CodecError::Message)?;
    output.extend_from_slice(&sector);
  }
  Ok(output)
}
