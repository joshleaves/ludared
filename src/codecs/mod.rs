use errors::CodecError;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;

#[derive(Clone, Default)]
pub(crate) struct ScriptEnvironment(pub(crate) HashMap<String, String>);

pub(crate) trait LudaredCodecArgs: Sized + DeserializeOwned {
  fn from_args(args: Option<&str>) -> Result<Self, serde_json::Error> {
    serde_json::from_str(args.unwrap_or("{}"))
  }

  /// Merges preset arguments with user-provided arguments.
  /// User-provided values take precedence over preset values.
  fn merge(preset: &str, user_args: Option<&str>) -> Result<String, serde_json::Error> {
    let mut merged: serde_json::Map<String, serde_json::Value> = serde_json::from_str(preset)?;

    let user: serde_json::Map<String, serde_json::Value> =
      serde_json::from_str(user_args.unwrap_or("{}"))?;

    merged.extend(user);

    serde_json::to_string(&merged)
  }
}

macro_rules! debug_result {
  ($label:expr => $expression:expr) => {{
    let result = $expression;
    let symbol = if result { "✓" } else { "✗" };
    ::log::debug!("  {symbol} {}", $label);
    result
  }};
}

pub mod cdrom;
pub mod errors;
pub mod generic;
pub mod nintendo;
pub mod registry;
pub mod scripts;
pub mod sega;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CodecHandlingConfidence {
  /// "I cannot decode that data meaningfully, no way."
  No,
  /// "I may extract something out of that data, maybe garbage, maybe gold."
  Possible,
  /// "All signs point to yes, but false positives are possible."
  Likely,
  /// "I know that data, I know that format, I'm in charge."
  Certain,
}

impl std::fmt::Display for CodecHandlingConfidence {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::No => write!(f, "No"),
      Self::Possible => write!(f, "Possible"),
      Self::Likely => write!(f, "Likely"),
      Self::Certain => write!(f, "Certain"),
    }
  }
}

/// How a codec reconstructs its input from decoded artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodecKind {
  /// Reconstructs the complete input without requiring its original bytes.
  Rebuild,

  /// Modifies the current input while preserving unmodified regions.
  Patch,
}

impl std::fmt::Display for CodecKind {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Rebuild => write!(f, "Rebuild"),
      Self::Patch => write!(f, "Patch"),
    }
  }
}

/// A named byte artifact produced by a codec.
///
/// Names identify artifacts within a decode operation and allow
/// the codec to associate them with their corresponding components
/// during encoding.
pub struct DecodedArtifact {
  pub name: String,
  pub data: Vec<u8>,
}

/// Converts binary data into named artifacts and back.
///
/// The codec owns artifact naming and binary transformations.
/// The core owns paths, caching, overrides, and build orchestration.
///
/// # Round-tripping
///
/// For a Rebuild codec:
/// `encode(&[], &decode(x), args) == x`
///
/// For a Patch codec:
/// `encode(&x, &decode(x), args) == x`
///
/// These identities are expected for supported inputs when artifacts
/// are unchanged. A codec must preserve or reconstruct any information
/// necessary to satisfy its round-trip contract.
pub trait Codec: Send + Sync {
  fn id(&self) -> &str;
  fn name(&self) -> &str;
  fn description(&self) -> &str;

  /// Returns the named argument presets available for this codec.
  /// Presets provide CLI shorthand (e.g. `codec+iso9660`).
  fn presets(&self) -> Vec<&str> {
    vec![]
  }

  /// Merges a named preset into the provided arguments.
  /// Explicit arguments take precedence over preset defaults.
  /// Returns an error by default if the preset is unsupported.
  fn preset_args(&self, preset: &str, _: Option<&str>) -> Result<String, CodecError> {
    Err(CodecError::UndefinedPreset(
      preset.to_owned(),
      self.id().to_owned(),
    ))
  }

  /// Which of the two encoding strategies this codec implements.
  ///
  /// Declared by the codec itself, so the core can tell the two apart from the
  /// registry rather than by downcasting or by reading anything into the id.
  fn kind(&self) -> CodecKind;

  /// Reports how confidently this codec recognizes the given input.
  ///
  /// Detection must not produce artifacts or have side effects.
  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError>;

  /// Suggests a name for a decode operation using this codec.
  ///
  /// The name identifies the DecodeNode in the manifest, not any of
  /// its output artifacts. It must be independent of the input bytes.
  ///
  /// `args` contains the codec-specific configuration and may be used
  /// to derive a meaningful name.
  ///
  /// The caller may override this suggestion with an explicit name.
  /// Name uniqueness and manifest constraints are enforced by the core.
  fn decode_name(&self, args: Option<&str>) -> Result<String, CodecError>;

  /// Decodes `data` into named artifacts using `args`.
  ///
  /// The same `args` are passed to `encode` for reconstruction.
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError>;

  /// Reconstructs the parent artifact from decoded artifacts.
  ///
  /// For `Rebuild` codecs, `original` is empty and the result is built
  /// entirely from `artifacts`.
  ///
  /// For `Patch` codecs, `original` contains the current parent bytes.
  /// The codec modifies and returns this buffer, preserving bytes outside
  /// the affected regions.
  ///
  /// Ownership of `original` is transferred intentionally, allowing codecs
  /// to modify the input buffer in place without copying it. This enables
  /// successive patches to reuse the same allocation.
  ///
  /// `args` must have the same meaning as in `decode`.
  /// Invalid or insufficient artifacts must produce a `CodecError`.
  #[allow(dead_code)]
  fn encode(
    &self,
    original: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError>;
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde::Deserialize;
  use serde_json::{Value, json};

  #[derive(Debug, Deserialize, PartialEq)]
  #[serde(deny_unknown_fields)]
  struct TestArgs {
    #[serde(default)]
    count: u32,
    #[serde(default)]
    format: String,
  }

  impl LudaredCodecArgs for TestArgs {}

  fn merge(preset: &str, user_args: Option<&str>) -> Value {
    let merged = TestArgs::merge(preset, user_args).unwrap();
    serde_json::from_str(&merged).unwrap()
  }

  #[test]
  fn merge_preset_only() {
    assert_eq!(
      merge(r#"{"format":"iso9660"}"#, None),
      json!({"format": "iso9660"})
    );
  }

  #[test]
  fn merge_with_additional_user_args() {
    assert_eq!(
      merge(r#"{"format":"iso9660"}"#, Some(r#"{"count":150}"#)),
      json!({"format": "iso9660", "count": 150})
    );
  }

  #[test]
  fn merge_user_args_override_preset() {
    assert_eq!(
      merge(
        r#"{"format":"iso9660","count":10}"#,
        Some(r#"{"format":"raw","count":0}"#)
      ),
      json!({"format": "raw", "count": 0})
    );
  }

  #[test]
  fn merge_empty_user_args() {
    assert_eq!(
      merge(r#"{"format":"iso9660"}"#, Some("{}")),
      merge(r#"{"format":"iso9660"}"#, None)
    );
  }

  #[test]
  fn merge_empty_preset() {
    assert_eq!(merge("{}", Some(r#"{"count":150}"#)), json!({"count": 150}));
  }

  #[test]
  fn merge_does_not_apply_serde_defaults() {
    let merged = TestArgs::merge(r#"{"format":"iso9660"}"#, None).unwrap();

    assert_eq!(
      serde_json::from_str::<Value>(&merged).unwrap(),
      json!({"format": "iso9660"})
    );

    // Defaults are applied only when deserializing into the codec args.
    let parsed = TestArgs::from_args(Some(&merged)).unwrap();

    assert_eq!(
      parsed,
      TestArgs {
        count: 0,
        format: "iso9660".to_owned(),
      }
    );
  }

  #[test]
  fn merge_rejects_invalid_preset_json() {
    assert!(TestArgs::merge("{invalid", None).is_err());
  }

  #[test]
  fn merge_rejects_invalid_user_json() {
    assert!(TestArgs::merge("{}", Some("{invalid")).is_err());
  }

  #[test]
  fn merge_rejects_non_object_json() {
    assert!(TestArgs::merge("[]", None).is_err());
    assert!(TestArgs::merge("{}", Some("[]")).is_err());
  }

  #[test]
  fn merge_is_shallow() {
    assert_eq!(
      merge(r#"{"nested":{"a":1,"b":2}}"#, Some(r#"{"nested":{"a":3}}"#)),
      json!({"nested": {"a": 3}})
    );
  }
}
