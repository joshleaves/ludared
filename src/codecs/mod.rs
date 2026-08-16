macro_rules! debug_result {
  ($label:expr => $expression:expr) => {{
    let result = $expression;
    let symbol = if result { "✓" } else { "✗" };
    ::log::debug!("  {symbol} {}", $label);
    result
  }};
}

pub mod errors;
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

pub trait Codec: Send + Sync {
  fn id(&self) -> &'static str;
  fn name(&self) -> &'static str;
  fn description(&self) -> &'static str;

  fn can_handle(&self, data: &[u8]) -> CodecHandlingConfidence;
}
