use std::path::PathBuf;
use std::{fmt::Display, process::ExitCode};

#[derive(Debug)]
pub(crate) enum AppError {
  CodecUnavailable(String),
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
}

impl Display for AppError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match &self {
      AppError::CodecUnavailable(s) => write!(f, "Unavailable codec: {}", s),
      AppError::ConfigurationFileAlreadyExists() => write!(f, "Configuration file already exists"),
      AppError::ConfigurationFileIo(e) => write!(f, "Could not open ludared.toml: {}", e),
      AppError::ConfigurationFileDeserialize(e) => {
        write!(f, "Could not parse ludared.toml:\n {}", e)
      }
      AppError::ConfigurationFileSerialize(e) => {
        write!(f, "Could not serialize ludared.toml:\n {}", e)
      }
      AppError::ManifestFileIo(e) => write!(f, "Could not open manifest: {}", e),
      AppError::ManifestFileJson(e) => write!(f, "Could not parse manifest:\n {}", e),
      AppError::NoSuchFile(p) => write!(f, "File missing: {}", p.display()),
      AppError::SourceAlreadyExists(p) => write!(f, "Source already exists: {}", p.display()),
      AppError::SourceFileIo(p, e) => write!(f, "Could not open '{}': {}", p.display(), e),
      AppError::SourceFileMismatch(p, expected, actual) => {
        write!(
          f,
          "Invalid SHA256 for source file '{}'\n  expected: {}\n       got: {}",
          p.display(),
          expected,
          actual
        )
      }
      AppError::SourceNotFound(p) => write!(f, "Source not found in manifest: {}", p.display()),
      AppError::CleanIo(p, e) => write!(f, "Could not clean {}: {}", p.display(), e),
      AppError::Io(e) => write!(f, "I/O Error ({})", e),
    }
  }
}

impl From<std::io::Error> for AppError {
  fn from(err: std::io::Error) -> Self {
    Self::Io(err)
  }
}

impl From<AppError> for ExitCode {
  fn from(_err: AppError) -> Self {
    ExitCode::FAILURE
  }
}
