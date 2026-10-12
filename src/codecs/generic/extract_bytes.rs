use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::LudaredCodecArgs;
use crate::codecs::errors::CodecError;
use serde::Deserialize;

const CODEC_ID: &str = "std/generic/extract_bytes";
const CODEC_NAME: &str = "Extract bytes";
const CODEC_DESC: &str = "Raw bytes extractor

Extracts a range of bytes from the input into a separate artifact.

Reassembling replaces that range of the artifact it was extracted from,
leaving every other byte alone, so several extractions may share one
input. The range must already fit inside the artifact: it is never grown
to fit and never truncated to fit.

Available arguments:
  target
    String, output artifact name.
  offset
    Numeric, byte offset at which extraction starts.
  length
    Numeric, number of bytes to extract.
";

pub(crate) struct ExtractBytes;

#[derive(Debug, Deserialize)]
pub(crate) struct ExtractBytesCodecArgs {
  pub(crate) target: String,
  pub(crate) offset: usize,
  pub(crate) length: usize,
}

impl LudaredCodecArgs for ExtractBytesCodecArgs {}

impl Codec for ExtractBytes {
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
    CodecKind::Patch
  }

  fn can_handle(&self, _: &[u8]) -> CodecHandlingConfidence {
    CodecHandlingConfidence::Certain
  }

  fn decode_name(&self, args: Option<&str>) -> Result<String, CodecError> {
    let args = ExtractBytesCodecArgs::from_args(args)?;
    Ok(match args.target.rsplit_once('.') {
      Some((name, _)) => name.to_owned(),
      None => args.target,
    })
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    let args = ExtractBytesCodecArgs::from_args(args)?;
    let from = args.offset;
    let to = args.offset.checked_add(args.length).ok_or_else(|| {
      CodecError::Message(format!(
        "Out of bounds extraction: {}+{} overflows",
        args.offset, args.length
      ))
    })?;
    if to > data.len() {
      return Err(CodecError::Message(format!(
        "Out of bounds extraction: {}-{} (Maximum: {})",
        from,
        to,
        data.len()
      )));
    }
    Ok(vec![DecodedArtifact {
      name: args.target,
      data: data[from..to].to_vec(),
    }])
  }

  /// Replaces the extracted range in `original` with the provided artifact.
  ///
  /// The replacement must match the configured length and fit entirely
  /// within `original`. All other bytes are preserved unchanged.
  fn encode(
    &self,
    mut original: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    let args = ExtractBytesCodecArgs::from_args(args)?;

    if artifacts.len() != 1 {
      return Err(CodecError::Message(format!(
        "Expected exactly one artifact ('{}'), found {}",
        args.target,
        artifacts.len()
      )));
    }

    let artifact = &artifacts[0];

    if artifact.name != args.target {
      return Err(CodecError::Message(format!(
        "Expected artifact '{}', found '{}'",
        args.target, artifact.name
      )));
    }

    if artifact.data.len() != args.length {
      return Err(CodecError::Message(format!(
        "Expected {} bytes for artifact '{}', found {}",
        args.length,
        args.target,
        artifact.data.len()
      )));
    }

    let end = args.offset.checked_add(args.length).ok_or_else(|| {
      CodecError::Message(format!(
        "Out of bounds replacement: {}+{} overflows",
        args.offset, args.length
      ))
    })?;

    if end > original.len() {
      return Err(CodecError::Message(format!(
        "Out of bounds replacement: {}-{} (Maximum: {})",
        args.offset,
        end,
        original.len()
      )));
    }

    original[args.offset..end].copy_from_slice(&artifact.data);

    Ok(original)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A parent long enough that an extraction into the middle of it leaves
  /// something on either side of the result.
  const SOURCE: &[u8] = b"SnES header, then ROM banks, then nothing much else";

  /// An extraction of `offset..offset + length`, as the codec is given it.
  fn args(target: &str, offset: usize, length: usize) -> String {
    serde_json::json!({
      "target": target,
      "offset": offset,
      "length": length
    })
    .to_string()
  }

  /// The single artifact a range of the parent would decode into.
  fn artifact(name: &str, data: &[u8]) -> DecodedArtifact {
    DecodedArtifact {
      name: name.to_owned(),
      data: data.to_vec(),
    }
  }

  #[test]
  fn patches_the_range_back_into_the_input_it_was_taken_from() {
    let args = args("tail.bin", 27, 5);
    let artifacts = ExtractBytes.decode(SOURCE, Some(&args)).unwrap();

    // Putting an extraction back over the input it came from changes nothing,
    // whatever range of the input it covers.
    assert_eq!(
      ExtractBytes
        .encode(SOURCE.to_vec(), &artifacts, Some(&args))
        .unwrap(),
      SOURCE
    );
  }

  #[test]
  fn replaces_the_requested_range_and_leaves_the_rest_of_the_input_alone() {
    let args = args("patch.bin", 27, 5);
    let replacement = b"PATCH";

    let patched = ExtractBytes
      .encode(
        SOURCE.to_vec(),
        &[artifact(
          "patch.bin",
          replacement,
        )],
        Some(&args),
      )
      .unwrap();

    // The bytes before and after the window are the input's own, untouched.
    assert_eq!(patched.len(), SOURCE.len());
    assert_eq!(&patched[..27], &SOURCE[..27]);
    assert_eq!(&patched[27..32], replacement);
    assert_eq!(&patched[32..], &SOURCE[32..]);
  }

  #[test]
  fn replaces_exactly_as_many_bytes_as_the_window_is_wide() {
    // A one-byte window in the middle of the input, so that the two bytes round
    // it are the only things that must survive.
    let args = args("one.bin", 1, 1);

    let patched = ExtractBytes
      .encode(
        SOURCE.to_vec(),
        &[artifact(
          "one.bin",
          b"X",
        )],
        Some(&args),
      )
      .unwrap();

    assert_eq!(
      patched,
      b"SXES header, then ROM banks, then nothing much else"
    );
  }

  #[test]
  fn patches_a_window_sitting_at_either_end_of_the_input() {
    // Neither end needs a guard of zeroes or a byte dropped to make room for
    // it.
    for (offset, length) in [
      (0, 4),
      (SOURCE.len() - 4, 4),
      (SOURCE.len(), 0),
      (0, 0),
    ] {
      let args = args("edge.bin", offset, length);
      let artifacts = ExtractBytes.decode(SOURCE, Some(&args)).unwrap();

      assert_eq!(
        ExtractBytes
          .encode(SOURCE.to_vec(), &artifacts, Some(&args))
          .unwrap(),
        SOURCE.to_vec(),
        "the window {offset}..{} should round-trip",
        offset + length
      );
    }
  }

  #[test]
  fn patches_one_range_at_a_time_so_several_may_share_an_input() {
    // Each patch is applied over the complete artifact the last one produced,
    // which is what makes the order they are applied in the order they were
    // declared in, and what lets the last of them win where two overlap.
    let head = args("head.bin", 0, 5);
    let tail = args("tail.bin", SOURCE.len() - 5, 5);

    let after_head = ExtractBytes
      .encode(
        SOURCE.to_vec(),
        &[artifact(
          "head.bin",
          b"HEAD!",
        )],
        Some(&head),
      )
      .unwrap();
    let after_tail = ExtractBytes
      .encode(
        after_head,
        &[artifact(
          "tail.bin",
          b"TAIL!",
        )],
        Some(&tail),
      )
      .unwrap();

    // The second patch left the first one alone: neither window overlaps.
    assert_eq!(
      after_tail,
      b"HEAD!header, then ROM banks, then nothing muchTAIL!"
    );
  }

  #[test]
  fn never_extends_an_input_to_fit_a_window_reaching_past_it() {
    // A window running off either end is refused rather than clipped to what is
    // there, and an artifact one byte short of that does not make it fit.
    for (offset, length) in [
      (SOURCE.len(), 1),
      (SOURCE.len() - 1, 2),
      (SOURCE.len() + 1, 0),
      (SOURCE.len() + 100, 0),
    ] {
      let args = args("beyond.bin", offset, length);

      let err = ExtractBytes
        .encode(
          SOURCE.to_vec(),
          &[artifact(
            "beyond.bin",
            &vec![0u8; length],
          )],
          Some(&args),
        )
        .unwrap_err();

      assert!(
        err.to_string().contains("Out of bounds replacement"),
        "the window {offset}..{offset}+{length} should be refused: {err}"
      );
    }
  }

  #[test]
  fn never_truncates_an_input_down_to_the_size_of_a_window() {
    // A window narrower than the input leaves the input's own length alone, so
    // the bytes after it are never dropped to make the result fit the window.
    let args = args("narrow.bin", 0, 1);

    let patched = ExtractBytes
      .encode(
        SOURCE.to_vec(),
        &[artifact(
          "narrow.bin",
          b"X",
        )],
        Some(&args),
      )
      .unwrap();

    assert_eq!(patched.len(), SOURCE.len());
    assert_eq!(&patched[1..], &SOURCE[1..]);
  }

  #[test]
  fn refuses_an_extraction_whose_end_overflows() {
    // The same guard `encode` keeps: `offset + length` running past the end of
    // the address space is a window that fits nowhere, not one that wraps back
    // round to the start of one and hands back a slice of the input's head.
    let args = args("overflow.bin", usize::MAX - 2, 8);

    // The error rather than the panic: `DecodedArtifact` is no `Debug`, so this
    // cannot go through `unwrap_err`.
    let Err(err) = ExtractBytes.decode(SOURCE, Some(&args)) else {
      panic!("an overflowing window should be refused");
    };

    assert!(
      err.to_string().contains("overflows"),
      "an overflowing window should be refused: {err}"
    );
  }

  #[test]
  fn refuses_a_window_whose_end_overflows() {
    // `offset + length` running past the end of the address space is a window
    // that fits nowhere, not one that wraps back round to the start of one.
    let args = args("overflow.bin", usize::MAX - 2, 8);

    let err = ExtractBytes
      .encode(
        SOURCE.to_vec(),
        &[artifact(
          "overflow.bin",
          &[0u8; 8],
        )],
        Some(&args),
      )
      .unwrap_err();

    assert!(
      err.to_string().contains("overflows"),
      "an overflowing window should be refused: {err}"
    );
  }

  #[test]
  fn refuses_anything_but_the_extraction_it_decodes() {
    let args = args("tail.bin", 27, 5);

    for artifacts in [
      // Nothing at all.
      vec![],
      // The right bytes under a name this codec never hands out.
      vec![artifact(
        "head.bin",
        &SOURCE[27..32],
      )],
      // The right name holding a window of the wrong length.
      vec![artifact(
        "tail.bin",
        &SOURCE[27..33],
      )],
      // A second artifact alongside the extraction.
      vec![
        artifact("tail.bin", &SOURCE[27..32]),
        artifact("head.bin", &SOURCE[..4]),
      ],
    ] {
      let names: Vec<&str> = artifacts
        .iter()
        .map(|artifact| artifact.name.as_str())
        .collect();

      assert!(
        ExtractBytes
          .encode(SOURCE.to_vec(), &artifacts, Some(&args))
          .is_err(),
        "{names:?} should not have been accepted"
      );
    }
  }
}
