use crate::errors::app_error::AppError;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Configuration {
  pub project: ProjectConfiguration,

  #[serde(default)]
  pub paths: PathsConfiguration,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ProjectConfiguration {
  pub name: String,
  pub manifest: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PathsConfiguration {
  #[serde(default = "default_sources_path")]
  pub sources: PathBuf,

  #[serde(default = "default_builds_path")]
  pub builds: PathBuf,

  #[serde(default = "default_cache_path")]
  pub cache: PathBuf,
}

impl Default for PathsConfiguration {
  fn default() -> Self {
    Self {
      sources: default_sources_path(),
      builds: default_builds_path(),
      cache: default_cache_path(),
    }
  }
}

fn default_sources_path() -> PathBuf {
  "sources".into()
}

fn default_builds_path() -> PathBuf {
  "builds".into()
}

fn default_cache_path() -> PathBuf {
  "builds/cache".into()
}

impl Configuration {
  pub fn load_default() -> Result<Self, AppError> {
    Self::load(Path::new("ludared.toml"))
  }

  pub fn load(path: &Path) -> Result<Self, AppError> {
    let content = std::fs::read_to_string(path).map_err(AppError::ConfigurationFileIo)?;
    let config = toml::from_str(&content).map_err(AppError::ConfigurationFileDeserialize)?;
    Ok(config)
  }

  pub fn save(&self, path: &Path) -> Result<(), AppError> {
    let toml = toml::to_string_pretty(self).map_err(AppError::ConfigurationFileSerialize)?;
    std::fs::write(path, toml).map_err(AppError::ConfigurationFileIo)?;
    Ok(())
  }
}
