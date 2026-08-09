use crate::errors::app_error::AppError;
use crate::source::Source;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct Manifest {
  #[serde(default)]
  pub sources: HashMap<PathBuf, Source>,
}

impl Manifest {
  pub(crate) fn load(path: &Path) -> Result<Self, AppError> {
    let content = std::fs::read_to_string(path).map_err(AppError::ManifestFileIo)?;
    // you may want a Json variant later
    let manifest: Self = serde_json::from_str(&content).map_err(AppError::ManifestFileJson)?;
    Ok(manifest)
  }

  pub(crate) fn save(&self, path: &Path) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(self).map_err(AppError::ManifestFileJson)?;
    std::fs::write(path, json).map_err(AppError::ManifestFileIo)?;
    Ok(())
  }
}
