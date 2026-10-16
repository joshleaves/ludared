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
}

macro_rules! debug_result {
  ($label:expr => $expression:expr) => {{
    let result = $expression;
    let symbol = if result { "✓" } else { "✗" };
    ::log::debug!("  {symbol} {}", $label);
    result
  }};
}

pub(crate) mod cdrom;
pub mod errors;
pub(crate) mod generic;
pub(crate) mod nintendo;
pub(crate) mod registry;
pub(crate) mod scripts;

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
