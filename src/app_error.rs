use std::{fmt::Display, process::ExitCode};

pub(crate) enum AppError {
  ConfigurationFileIo(std::io::Error),
  ConfigurationFileToml(toml::de::Error),

  Io(std::io::Error),
  Toml(toml::de::Error),
}

impl Display for AppError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match &self {
      AppError::ConfigurationFileIo(e) => write!(f, "Could not open ludared.toml: {}", e),
      AppError::ConfigurationFileToml(e) => write!(f, "Could not parse ludared.toml:\n {}", e),
      AppError::Io(e) => write!(f, "I/O Error ({})", e),
      AppError::Toml(e) => write!(f, "TOML Error ({}", e),
    }
  }
}

impl Into<ExitCode> for AppError {
  fn into(self) -> ExitCode {
    ExitCode::FAILURE
  }
}

impl From<std::io::Error> for AppError {
  fn from(e: std::io::Error) -> Self {
    AppError::Io(e)
  }
}

impl From<toml::de::Error> for AppError {
  fn from(e: toml::de::Error) -> Self {
    AppError::Toml(e)
  }
}
