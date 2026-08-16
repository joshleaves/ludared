use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub(crate) enum VirtualPathError {
  #[error("Empty path")]
  EmptyPath,

  #[error("Path begins with a slash: {0}")]
  StartsWithSlash(String),

  #[error("Path ends with a slash: {0}")]
  EndsWithSlash(String),

  #[error("Path contains empty component(s): {0}")]
  EmptyComponent(String),

  #[error("Path contains ./..: {0}")]
  ContainsReferenceComponent(String),

  #[error("Path contains backslash character: {0}")]
  ContainsBackslash(String),

  #[error("Missing file: {}", .0.display())]
  MissingFile(PathBuf),
}
