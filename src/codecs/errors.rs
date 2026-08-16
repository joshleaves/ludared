use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodecError {
  #[error("{0}")]
  Message(String),

  #[error("Could not parse JSON args:\n{0}")]
  JsonArgs(#[from] serde_json::Error),
}
