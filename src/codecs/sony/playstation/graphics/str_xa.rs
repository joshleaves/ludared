//! Logical PlayStation STR/XA sector demultiplexer.
//!
//! This codec operates on the *cooked* payload stream produced by the outer
//! CD-ROM Mode 2 XA / ISO 9660 codecs, NOT on raw 2352-byte CD sectors.
//!
//! Outputs, in order:
//!   layout.bin  - one byte per sector: 0=padding, 1=Form1 video, 2=Form2 audio
//!   video.str   - concatenated 2048-byte Form1 STR video sectors
//!   audio.xa    - concatenated 2324-byte Form2 XA audio sector payloads
//! Padding sectors are assumed to contain 0x00 and are regenerated as zeros
//! during encoding; they do not produce a separate artifact.
//!
//! The source has no XA subheaders: classification is necessarily heuristic.
//! This implementation targets the observed Fuuraiki convention: Form1 video
//! sectors begin with 60 01 01 80, padding sectors are all zero, and remaining
//! sectors are Form2 audio. It rejects ambiguous/truncated inputs rather than
//! silently generating a misleading layout. It is NOT a generic STR parser.
//!
//! Rebuilding uses the three artifacts and zero-fills padding sectors. Sector
//! counts are fixed by layout.bin; modifying a stream must preserve its number
//! of sectors. Changes to the outer XA sector subheaders are outside this
//! codec's scope.

use crate::codecs::errors::CodecError;
use crate::codecs::{Codec, CodecHandlingConfidence, CodecKind, DecodedArtifact};

pub(crate) struct StrXa;

const VIDEO_SIZE: usize = 2048;
const AUDIO_SIZE: usize = 2324;
const MAGIC: [u8; 4] = [
  0x60,
  0x01,
  0x01,
  0x80,
];
const PADDING: u8 = 0;
const VIDEO: u8 = 1;
const AUDIO: u8 = 2;

const CODEC_ID: &str = "std/sony/playstation/graphics/str_xa";
const CODEC_NAME: &str = "PlayStation STR/XA demultiplexer";
const CODEC_DESC: &str = r#"PlayStation STR/XA cooked-sector demultiplexer.

Input: concatenated Mode 2 XA user-data payloads, with 2048-byte Form1 video
sectors and 2324-byte Form2 audio sectors, as emitted by the outer CD-ROM codec.
This is NOT a raw 2352-byte STR file.

Artifacts:
  layout.bin   one u8 per original sector: 0=padding, 1=video, 2=audio
  video.str    all 2048-byte video sectors in original order
  audio.xa     all 2324-byte audio payloads in original order

Padding sectors are assumed to be zero-filled and are regenerated as 0x00.

Detection is Fuuraiki-specific: 60 01 01 80 marks video, an all-zero 2048-byte
sector marks padding, and other 2324-byte payloads are treated as audio. Because
XA subheaders were removed by the outer codec, this cannot distinguish every
possible Form1 payload from Form2 audio. Do not apply to arbitrary STR files
without verifying their conventions.

The layout is an internal Ludared binary format, not a Sony standard. Encoding
requires exact sector-aligned stream lengths matching the layout; it does not
reflow sectors or modify the outer .STR.XA metadata. No arguments.
"#;

fn error(message: impl Into<String>) -> CodecError {
  CodecError::Message(message.into())
}

fn is_video(data: &[u8]) -> bool {
  data.starts_with(&MAGIC)
}

fn is_padding(data: &[u8]) -> bool {
  data.len() >= VIDEO_SIZE && data[..VIDEO_SIZE].iter().all(|&b| b == 0)
}

fn split(data: &[u8]) -> Result<Vec<DecodedArtifact>, CodecError> {
  let mut layout = Vec::new();
  let mut video = Vec::new();
  let mut audio = Vec::new();
  let mut offset = 0usize;

  while offset < data.len() {
    let rest = &data[offset..];
    let (kind, size) = if rest.len() >= VIDEO_SIZE && is_video(rest) {
      (VIDEO, VIDEO_SIZE)
    } else if is_padding(rest) {
      (PADDING, VIDEO_SIZE)
    } else if rest.len() >= AUDIO_SIZE {
      (AUDIO, AUDIO_SIZE)
    } else {
      return Err(error(format!(
        "Unrecognized or truncated STR/XA sector at byte {offset} ({} bytes remain)",
        rest.len()
      )));
    };

    let sector = &rest[..size];
    match kind {
      VIDEO => video.extend_from_slice(sector),
      AUDIO => audio.extend_from_slice(sector),
      PADDING => {}
      _ => unreachable!(),
    }
    layout.push(kind);
    offset += size;
  }

  if !layout.contains(&VIDEO) || !layout.contains(&AUDIO) {
    return Err(error(
      "Not a recognized interleaved STR/XA video/audio stream",
    ));
  }

  Ok(vec![
    DecodedArtifact {
      name: "layout.bin".into(),
      data: layout,
    },
    DecodedArtifact {
      name: "video.str".into(),
      data: video,
    },
    DecodedArtifact {
      name: "audio.xa".into(),
      data: audio,
    },
  ])
}

fn join(artifacts: &[DecodedArtifact]) -> Result<Vec<u8>, CodecError> {
  if artifacts.len() != 3 {
    return Err(error(format!(
      "Expected exactly three STR/XA artifacts, got {}",
      artifacts.len()
    )));
  }

  let get = |name: &str| -> Result<&[u8], CodecError> {
    let mut matches = artifacts.iter().filter(|artifact| artifact.name == name);
    let artifact = matches
      .next()
      .ok_or_else(|| error(format!("Missing artifact: {name}")))?;
    if matches.next().is_some() {
      return Err(error(format!("Duplicate artifact: {name}")));
    }
    Ok(&artifact.data)
  };

  let layout = get("layout.bin")?;
  let video = get("video.str")?;
  let audio = get("audio.xa")?;

  let mut counts = [0usize; 3];
  for (index, &kind) in layout.iter().enumerate() {
    let count = counts
      .get_mut(kind as usize)
      .ok_or_else(|| error(format!("Invalid layout byte {kind} at sector {index}")))?;
    *count += 1;
  }

  for (name, actual, count, size) in [
    ("video.str", video.len(), counts[1], VIDEO_SIZE),
    ("audio.xa", audio.len(), counts[2], AUDIO_SIZE),
  ] {
    let expected = count
      .checked_mul(size)
      .ok_or_else(|| error("Sector count overflow"))?;
    if actual != expected {
      return Err(error(format!(
        "{name}: expected {expected} bytes from layout.bin, got {actual}"
      )));
    }
  }

  let padding_size = counts[0]
    .checked_mul(VIDEO_SIZE)
    .ok_or_else(|| error("Sector count overflow"))?;
  let capacity = padding_size
    .checked_add(video.len())
    .and_then(|size| size.checked_add(audio.len()))
    .ok_or_else(|| error("Output size overflow"))?;
  let mut result = Vec::with_capacity(capacity);
  let mut offsets = [0usize; 3];
  let streams = [
    &[][..],
    video,
    audio,
  ];
  let sizes = [
    0,
    VIDEO_SIZE,
    AUDIO_SIZE,
  ];

  for &kind in layout {
    let index = kind as usize; // validated above
    if index == PADDING as usize {
      result.resize(result.len() + VIDEO_SIZE, 0);
      continue;
    }
    let start = offsets[index];
    let end = start + sizes[index];
    result.extend_from_slice(&streams[index][start..end]);
    offsets[index] = end;
  }
  Ok(result)
}

impl Codec for StrXa {
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
    if data.starts_with(&MAGIC) && split(data).is_ok() {
      Ok(CodecHandlingConfidence::Certain)
    } else {
      Ok(CodecHandlingConfidence::No)
    }
  }

  fn decode_name(&self, _: Option<&str>) -> Result<String, CodecError> {
    Ok("str_xa".into())
  }

  fn decode(&self, data: &[u8], _: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    split(data)
  }

  fn encode(
    &self,
    _: Vec<u8>,
    artifacts: &[DecodedArtifact],
    _: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    join(artifacts)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sample() -> Vec<u8> {
    let mut data = vec![0x33; VIDEO_SIZE];
    data[..4].copy_from_slice(&MAGIC);
    data.extend(vec![0x5a; AUDIO_SIZE]);
    data.extend(vec![0; VIDEO_SIZE]);
    data.extend(vec![0x44; VIDEO_SIZE]);
    let last = data.len() - VIDEO_SIZE;
    data[last..last + 4].copy_from_slice(&MAGIC);
    data
  }

  #[test]
  fn round_trip() {
    let source = sample();
    let artifacts = split(&source).unwrap();
    assert_eq!(artifacts.len(), 3);
    assert_eq!(
      artifacts
        .iter()
        .map(|artifact| artifact.name.as_str())
        .collect::<Vec<_>>(),
      [
        "layout.bin",
        "video.str",
        "audio.xa"
      ]
    );
    assert_eq!(artifacts[0].data, [1, 2, 0, 1]);
    assert_eq!(join(&artifacts).unwrap(), source);
  }

  #[test]
  fn refuses_modified_sector_counts() {
    let mut artifacts = split(&sample()).unwrap();
    artifacts[1].data.pop();
    assert!(join(&artifacts).is_err());
  }

  #[test]
  fn refuses_unknown_layout_value() {
    let mut artifacts = split(&sample()).unwrap();
    artifacts[0].data[0] = 3;
    assert!(join(&artifacts).is_err());
  }

  #[test]
  fn refuses_truncation() {
    let mut source = sample();
    source.pop();
    assert!(split(&source).is_err());
  }
}
