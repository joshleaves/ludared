use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodecError {
  #[error("{0}")]
  Message(String),

  #[error("Could not parse options:\n{0}")]
  OptionsJson(#[from] serde_json::Error),
}
