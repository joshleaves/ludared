use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub(crate) enum AppError {
  // Domain-specific errors
  #[error("I/O Error: {0}")]
  Io(#[from] std::io::Error),

  #[error("Codec Error: {0}")]
  CodecError(#[from] crate::codecs::errors::CodecError),

  #[error("Virtual Path Error: {0}")]
  VirtualPathError(#[from] crate::virtual_path::errors::VirtualPathError),

  #[error("Unavailable codec {0}")]
  CodecUnavailable(String),

  #[error("Incompatible codec: {0}. Use --force to continue anyway.")]
  CodecIncompatible(String),

  #[error("Configuration file already exists")]
  ConfigurationFileAlreadyExists(),

  #[error("Could not open ludared.toml: {0}")]
  ConfigurationFileIo(std::io::Error),

  #[error("Could not parse ludared.toml:\n{0}")]
  ConfigurationFileDeserialize(toml::de::Error),

  #[error("Could not serialize ludared.toml:\n{0}")]
  ConfigurationFileSerialize(toml::ser::Error),

  #[error("Could not open manifest: {0}")]
  ManifestFileIo(std::io::Error),

  #[error("Could not parse manifest:\n {0}")]
  ManifestFileJson(serde_json::Error),

  #[error("Source already exists: {}", .0.display())]
  SourceAlreadyExists(PathBuf),

  #[error("Could not open '{}': {}", .0.display(), .1)]
  SourceFileIo(PathBuf, std::io::Error),

  #[error("Invalid SHA256 for source file '{}'\n  expected: {}\n       got: {}", .0.display(), .1, .2)]
  SourceFileMismatch(PathBuf, String, String),

  #[error("Source not found in manifest: {}", .0.display())]
  SourceNotFound(PathBuf),

  #[error("File missing: {}", .0.display())]
  NoSuchFile(PathBuf),

  #[error("Could not clean {}: {}", .0.display(), .1)]
  CleanIo(PathBuf, std::io::Error),
}
