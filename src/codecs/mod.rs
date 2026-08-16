use errors::CodecError;
use serde::de::DeserializeOwned;

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

pub mod errors;
pub(crate) mod generic;
pub(crate) mod registry;
pub(crate) mod snes;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CodecHandlingConfidence {
  No,
  Possible,
  Likely,
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

pub struct DecodedArtifact {
  pub name: String,
  pub data: Vec<u8>,
}

pub trait Codec: Send + Sync {
  fn id(&self) -> &'static str;
  fn name(&self) -> &'static str;
  fn description(&self) -> &'static str;

  fn can_handle(&self, data: &[u8]) -> CodecHandlingConfidence;

  fn decode_name(&self, args: Option<&str>) -> Result<String, CodecError>;
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError>;
}
