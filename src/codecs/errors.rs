use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodecError {
  #[error(transparent)]
  Io(#[from] std::io::Error),

  #[error(transparent)]
  Lua(#[from] mlua::Error),

  #[error("Invalid output name: empty")]
  ArtifactEmptyName,

  #[error("Duplicate artifact output name: {0}")]
  DuplicateArtifact(String),

  #[error("Invalid output name: {0} must not start with '/'")]
  ArtifactStartsWithSlash(String),

  #[error("Invalid output name: {0} must not end with '/'")]
  ArtifactEndsWithSlash(String),

  #[error("Invalid output name: {0} contains invalid sequence '{1}'")]
  ArtifactContainsInvalidSequence(String, String),

  #[error("Overlapping artifacts: {0} and {1}")]
  OverlappingArtifacts(String, String),

  #[error("Undefined preset '{0}' for codec '{1}'")]
  UndefinedPreset(String, String),

  #[error("{0}")]
  Message(String),

  #[error("Could not parse JSON args:\n{0}")]
  JsonArgs(#[from] serde_json::Error),

  #[error(transparent)]
  Script(#[from] crate::scripts::errors::ScriptError),
}
