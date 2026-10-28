use thiserror::Error;

#[derive(Error, Debug)]
pub enum ManifestError {
  #[error("Could not resolve virtual path: {0}")]
  CouldNotResolve(String),

  #[error("No decode found at '{0}'")]
  NoDecode(String),

  #[error("Multiple decodes found at '{0}'; specify a name")]
  DecodeNameRequired(String),

  #[error("Decode '{0}' not found at '{1}'")]
  DecodeNotFound(String, String),

  #[error("Cannot remove decode '{0}' at '{1}': it has children")]
  DecodeNotTerminal(String, String),

  #[error("Duplicate decode name: {0} under {1}")]
  DuplicateDecodeName(String, String),

  #[error("Duplicate output name: {0} under {1}")]
  DuplicateOutput(String, String),

  #[error("Overlapping artifacts: {0} and {1}")]
  OverlappingArtifacts(String, String),

  #[error("Decode '{0}' conflicts with existing decodes under '{1}'")]
  ConflictingDecodeKind(String, String),

  #[error("Decode '{0}' under '{1}' records output '{2}', but its codec did not produce it")]
  MissingOutput(String, String, String),

  #[error("Decode '{0}' under '{1}' produced unexpected output '{2}'")]
  UnexpectedOutput(String, String, String),
}
