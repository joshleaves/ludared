use crate::app_error::AppError;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub(crate) struct Manifest {
  #[serde(default)]
  pub sources: HashMap<PathBuf, Source>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Source {
  pub sha256: String,
  #[serde(default)]
  pub size: Option<u64>,
  #[serde(default)]
  pub label: Option<String>,
}

impl Manifest {
  pub(crate) fn load(path: &Path) -> Result<Self, AppError> {
    let content = std::fs::read_to_string(path).map_err(AppError::ManifestFileIo)?;
    // you may want a Json variant later
    let manifest: Self = serde_json::from_str(&content).map_err(AppError::ManifestFileJson)?;
    Ok(manifest)
  }
}
