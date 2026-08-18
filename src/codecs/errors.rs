use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodecError {
  #[error("Duplicate artifact output name: {0}")]
  DuplicateArtifact(String),

  #[error("{0}")]
  Message(String),

  #[error("Could not parse JSON args:\n{0}")]
  JsonArgs(#[from] serde_json::Error),
}
