use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use serde::Deserialize;

const CODEC_ID: &str = "std/generic/split";
const CODEC_NAME: &str = "Split bytes";
const CODEC_DESC: &str = r#"Split bytes at specified offsets

Splits the entire input into consecutive artifacts. The offsets mark the
start of each artifact after the first. Reassembly concatenates the artifacts
in their original order, allowing their sizes to change.

Available arguments:
  offsets
    Non-empty array of strictly increasing byte offsets. Each offset must be
    greater than zero and less than the input length.
  pattern
    Optional artifact filename pattern with exactly one %d or %0Nd placeholder.
    Defaults to part_%d.bin.

Example:
  {"offsets":[42,128],"pattern":"track_%02d.data"}
"#;

const DEFAULT_PATTERN: &str = "part_%d.bin";

pub(crate) struct Split;

#[derive(Debug, Deserialize)]
struct RawSplitCodecArgs {
  offsets: Vec<usize>,
  pattern: Option<String>,
}

impl LudaredCodecArgs for RawSplitCodecArgs {}

struct SplitPattern {
  prefix: String,
  width: usize,
  suffix: String,
}

impl SplitPattern {
  fn parse(pattern: &str) -> Result<Self, CodecError> {
    let invalid = || {
      CodecError::Message(
        "Split pattern must contain exactly one %d or %0Nd placeholder".to_owned(),
      )
    };

    let start = pattern.find('%').ok_or_else(invalid)?;
    let tail = &pattern[start + 1..];

    let (width, suffix) = if let Some(suffix) = tail.strip_prefix('d') {
      (0, suffix)
    } else if let Some(rest) = tail.strip_prefix('0') {
      let digits = rest.bytes().take_while(u8::is_ascii_digit).count();

      if digits == 0 || rest.as_bytes().get(digits) != Some(&b'd') {
        return Err(invalid());
      }

      let width = rest[..digits]
        .parse::<usize>()
        .map_err(|_| CodecError::Message("Invalid split pattern width".to_owned()))?;

      if width == 0 || width > 32 {
        return Err(CodecError::Message(
          "Split pattern width must be between 1 and 32".to_owned(),
        ));
      }

      (width, &rest[digits + 1..])
    } else {
      return Err(invalid());
    };

    if suffix.contains('%') {
      return Err(invalid());
    }

    Ok(Self {
      prefix: pattern[..start].to_owned(),
      width,
      suffix: suffix.to_owned(),
    })
  }

  fn format(&self, index: usize) -> String {
    format!(
      "{}{index:0width$}{}",
      self.prefix,
      self.suffix,
      width = self.width
    )
  }
}

struct SplitCodecArgs {
  offsets: Vec<usize>,
  pattern: SplitPattern,
}

impl SplitCodecArgs {
  fn build_from_args(args: Option<&str>) -> Result<Self, CodecError> {
    let raw = RawSplitCodecArgs::from_args(args)?;

    if raw.offsets.is_empty() {
      return Err(CodecError::Message(
        "Split requires at least one offset".to_owned(),
      ));
    }

    if raw.offsets[0] == 0 || raw.offsets.windows(2).any(|pair| pair[0] >= pair[1]) {
      return Err(CodecError::Message(
        "Split offsets must be strictly increasing and greater than zero".to_owned(),
      ));
    }

    let pattern = SplitPattern::parse(raw.pattern.as_deref().unwrap_or(DEFAULT_PATTERN))?;

    Ok(Self {
      offsets: raw.offsets,
      pattern,
    })
  }
}

impl Codec for Split {
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

  fn can_handle(&self, _: &[u8]) -> CodecHandlingConfidence {
    CodecHandlingConfidence::Possible
  }

  fn decode_name(&self, args: Option<&str>) -> Result<String, CodecError> {
    SplitCodecArgs::build_from_args(args)?;
    Ok("split".to_owned())
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    let args = SplitCodecArgs::build_from_args(args)?;

    if args
      .offsets
      .last()
      .is_some_and(|&offset| offset >= data.len())
    {
      return Err(CodecError::Message(
        "Split offsets must be inside the input".to_owned(),
      ));
    }

    let mut outputs = Vec::with_capacity(args.offsets.len() + 1);
    let mut start = 0;

    for (index, end) in args
      .offsets
      .iter()
      .copied()
      .chain(std::iter::once(data.len()))
      .enumerate()
    {
      outputs.push(DecodedArtifact {
        name: args.pattern.format(index),
        data: data[start..end].to_vec(),
      });

      start = end;
    }

    Ok(outputs)
  }

  fn encode(
    &self,
    _original: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    let args = SplitCodecArgs::build_from_args(args)?;

    if artifacts.len() != args.offsets.len() + 1 {
      return Err(CodecError::Message(format!(
        "Expected {} artifacts, found {}",
        args.offsets.len() + 1,
        artifacts.len()
      )));
    }

    let mut output = Vec::new();

    for (index, artifact) in artifacts.iter().enumerate() {
      let expected = args.pattern.format(index);

      if artifact.name != expected {
        return Err(CodecError::Message(format!(
          "Expected artifact '{expected}', found '{}'",
          artifact.name
        )));
      }

      output.extend_from_slice(&artifact.data);
    }

    Ok(output)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const SOURCE: &[u8] = b"HEADBODYTAIL";

  fn args(offsets: &[usize], pattern: Option<&str>) -> String {
    serde_json::json!({
      "offsets": offsets,
      "pattern": pattern
    })
    .to_string()
  }

  fn artifact(name: &str, data: &[u8]) -> DecodedArtifact {
    DecodedArtifact {
      name: name.to_owned(),
      data: data.to_vec(),
    }
  }

  #[test]
  fn produces_one_more_output_than_offsets() {
    for offsets in [
      &[4][..],
      &[4, 8][..],
      &[2, 4, 8][..],
    ] {
      let args = args(offsets, None);
      let outputs = Split.decode(SOURCE, Some(&args)).unwrap();

      assert_eq!(outputs.len(), offsets.len() + 1);
    }
  }

  #[test]
  fn splits_at_offsets_and_uses_default_names() {
    let args = args(&[4, 8], None);
    let outputs = Split.decode(SOURCE, Some(&args)).unwrap();

    assert_eq!(outputs[0].name, "part_0.bin");
    assert_eq!(outputs[0].data, b"HEAD");

    assert_eq!(outputs[1].name, "part_1.bin");
    assert_eq!(outputs[1].data, b"BODY");

    assert_eq!(outputs[2].name, "part_2.bin");
    assert_eq!(outputs[2].data, b"TAIL");
  }

  #[test]
  fn supports_zero_padded_patterns() {
    let args = args(&[4, 8], Some("track_%02d.data"));
    let outputs = Split.decode(SOURCE, Some(&args)).unwrap();

    assert_eq!(
      outputs.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
      [
        "track_00.data",
        "track_01.data",
        "track_02.data"
      ]
    );
  }

  #[test]
  fn reassembles_original_bytes() {
    let args = args(&[4, 8], None);
    let outputs = Split.decode(SOURCE, Some(&args)).unwrap();

    assert_eq!(
      Split.encode(Vec::new(), &outputs, Some(&args)).unwrap(),
      SOURCE
    );
  }

  #[test]
  fn reassembles_resized_artifacts() {
    let args = args(&[4, 8], None);

    let outputs = [
      artifact("part_0.bin", b"H"),
      artifact("part_1.bin", b"LONGER"),
      artifact("part_2.bin", b"T"),
    ];

    assert_eq!(
      Split.encode(Vec::new(), &outputs, Some(&args)).unwrap(),
      b"HLONGERT"
    );
  }

  #[test]
  fn rejects_empty_offsets() {
    let args = args(&[], None);

    assert!(Split.decode(SOURCE, Some(&args)).is_err());
  }

  #[test]
  fn rejects_zero_duplicate_and_unsorted_offsets() {
    for offsets in [
      &[0][..],
      &[4, 4][..],
      &[8, 4][..],
    ] {
      let args = args(offsets, None);

      assert!(Split.decode(SOURCE, Some(&args)).is_err());
    }
  }

  #[test]
  fn rejects_out_of_bounds_offsets() {
    for offset in [
      SOURCE.len(),
      SOURCE.len() + 1,
    ] {
      let args = args(&[offset], None);

      assert!(Split.decode(SOURCE, Some(&args)).is_err());
    }
  }

  #[test]
  fn rejects_patterns_without_usable_placeholder() {
    for pattern in [
      "track.data",
      "track_%s.data",
      "track_%d_%d.data",
      "track_%02x.data",
      "track_%0d.data",
      "track_%%d.data",
    ] {
      let args = args(&[4], Some(pattern));

      assert!(
        Split.decode(SOURCE, Some(&args)).is_err(),
        "pattern: {pattern}"
      );
    }
  }

  #[test]
  fn rejects_missing_extra_or_misnamed_artifacts() {
    let args = args(&[4], None);

    for outputs in [
      vec![],
      vec![artifact(
        "part_0.bin",
        b"HEAD",
      )],
      vec![
        artifact("part_0.bin", b"HEAD"),
        artifact("part_1.bin", b"BODYTAIL"),
        artifact("part_2.bin", b"EXTRA"),
      ],
      vec![
        artifact("wrong.bin", b"HEAD"),
        artifact("part_1.bin", b"BODYTAIL"),
      ],
    ] {
      assert!(Split.encode(Vec::new(), &outputs, Some(&args)).is_err());
    }
  }
}
