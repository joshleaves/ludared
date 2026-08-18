use thiserror::Error;

#[derive(Error, Debug)]
pub(crate) enum ManifestError {
  #[error("Could not resolve virtual path: {0}")]
  CouldNotResolve(String),

  #[error("Duplicate artifact output name: {0}")]
  DuplicateArtifact(String),

  #[error("Duplicate decode name: {0} under {1}")]
  DuplicateDecodeName(String, String),

  #[error("Duplicate output name: {0} under {1}")]
  DuplicateOutput(String, String),
}
