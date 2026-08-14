use std::fmt::Display;
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) enum VirtualPathError {
  EmptyPath,
  StartsWithSlash(String),
  EndsWithSlash(String),
  EmptyComponent(String),
  ContainsReferenceComponent(String),
  ContainsBackslash(String),
  MissingFile(PathBuf),
}

impl Display for VirtualPathError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match &self {
      Self::EmptyPath => write!(f, "Empty path"),
      Self::StartsWithSlash(s) => write!(f, "Path begins with a slash: {}", s),
      Self::EndsWithSlash(s) => write!(f, "Path ends with a slash: {}", s),
      Self::EmptyComponent(s) => write!(f, "Path contains empty component(s): {}", s),
      Self::ContainsReferenceComponent(s) => write!(f, "Path contains ./..: {}", s),
      Self::ContainsBackslash(s) => write!(f, "Path contains backslash character: {}", s),
      Self::MissingFile(p) => write!(f, "Missing file: {}", p.display()),
    }
  }
}
