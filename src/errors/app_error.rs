use crate::virtual_path::errors::VirtualPathError;
use std::path::PathBuf;
use std::{fmt::Display, process::ExitCode};

#[derive(Debug)]
pub(crate) enum AppError {
  CodecUnavailable(String),
  CodecIncompatible(String),
  ConfigurationFileAlreadyExists(),
  ConfigurationFileIo(std::io::Error),
  ConfigurationFileDeserialize(toml::de::Error),
  ConfigurationFileSerialize(toml::ser::Error),
  ManifestFileIo(std::io::Error),
  ManifestFileJson(serde_json::Error),
  SourceAlreadyExists(PathBuf),
  SourceFileIo(PathBuf, std::io::Error),
  SourceFileMismatch(PathBuf, String, String),
  SourceNotFound(PathBuf),
  NoSuchFile(PathBuf),
  CleanIo(PathBuf, std::io::Error),
  Io(std::io::Error),
  VirtualPathError(crate::virtual_path::errors::VirtualPathError),
}

impl Display for AppError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match &self {
      Self::CodecUnavailable(s) => write!(f, "Unavailable codec: {}", s),
      Self::CodecIncompatible(s) => write!(
        f,
        "Incompatible codec: {}. Use --force to continue anyway.",
        s
      ),
      Self::ConfigurationFileAlreadyExists() => write!(f, "Configuration file already exists"),
      Self::ConfigurationFileIo(e) => write!(f, "Could not open ludared.toml: {}", e),
      Self::ConfigurationFileDeserialize(e) => {
        write!(f, "Could not parse ludared.toml:\n {}", e)
      }
      Self::ConfigurationFileSerialize(e) => {
        write!(f, "Could not serialize ludared.toml:\n {}", e)
      }
      Self::ManifestFileIo(e) => write!(f, "Could not open manifest: {}", e),
      Self::ManifestFileJson(e) => write!(f, "Could not parse manifest:\n {}", e),
      Self::NoSuchFile(p) => write!(f, "File missing: {}", p.display()),
      Self::SourceAlreadyExists(p) => write!(f, "Source already exists: {}", p.display()),
      Self::SourceFileIo(p, e) => write!(f, "Could not open '{}': {}", p.display(), e),
      Self::SourceFileMismatch(p, expected, actual) => {
        write!(
          f,
          "Invalid SHA256 for source file '{}'\n  expected: {}\n       got: {}",
          p.display(),
          expected,
          actual
        )
      }
      Self::SourceNotFound(p) => write!(f, "Source not found in manifest: {}", p.display()),
      Self::CleanIo(p, e) => write!(f, "Could not clean {}: {}", p.display(), e),
      Self::Io(e) => write!(f, "I/O Error ({})", e),
      Self::VirtualPathError(e) => write!(f, "Virtual Path Error ({})", e),
    }
  }
}

impl From<std::io::Error> for AppError {
  fn from(err: std::io::Error) -> Self {
    Self::Io(err)
  }
}

impl From<VirtualPathError> for AppError {
  fn from(err: VirtualPathError) -> Self {
    Self::VirtualPathError(err)
  }
}

impl From<AppError> for ExitCode {
  fn from(_err: AppError) -> Self {
    ExitCode::FAILURE
  }
}
