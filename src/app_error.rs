use std::path::PathBuf;
use std::{fmt::Display, process::ExitCode};

pub(crate) enum AppError {
  ConfigurationFileIo(std::io::Error),
  ConfigurationFileToml(toml::de::Error),
  ManifestFileIo(std::io::Error),
  ManifestFileJson(serde_json::Error),
  SourceFileIo(PathBuf, std::io::Error),
  SourceFileMismatch(PathBuf, String, String),
  CleanIo(PathBuf, std::io::Error),
  // Io(std::io::Error),
}

impl Display for AppError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match &self {
      AppError::ConfigurationFileIo(e) => write!(f, "Could not open ludared.toml: {}", e),
      AppError::ConfigurationFileToml(e) => write!(f, "Could not parse ludared.toml:\n {}", e),
      AppError::ManifestFileIo(e) => write!(f, "Could not open manifest: {}", e),
      AppError::ManifestFileJson(e) => write!(f, "Could not parse manifest:\n {}", e),
      AppError::SourceFileIo(p, e) => write!(f, "Could not open '{}: {}", p.display(), e),
      AppError::SourceFileMismatch(p, expected, actual) => {
        write!(
          f,
          "Invalid SHA256 for source file '{}'\n  expected: {}\n       got: {}",
          p.display(),
          expected,
          actual
        )
      }
      AppError::CleanIo(p, e) => write!(f, "Could not clean {}: {}", p.display(), e), // AppError::Io(e) => write!(f, "I/O Error ({})", e),
    }
  }
}

impl From<AppError> for ExitCode {
  fn from(_err: AppError) -> Self {
    ExitCode::FAILURE
  }
}

// impl From<std::io::Error> for AppError {
//   fn from(e: std::io::Error) -> Self {
//     AppError::Io(e)
//   }
// }
