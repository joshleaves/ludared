use crate::errors::app_error::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Source {
  pub sha256: String,
  #[serde(default)]
  pub size: Option<u64>,
  #[serde(default)]
  pub label: Option<String>,
}

impl Source {
  pub(crate) fn new_from_file(source_path: &PathBuf) -> Result<Self, AppError> {
    let sha256 = Source::compute_sha256(source_path)?;
    let metadata =
      std::fs::metadata(source_path).map_err(|e| AppError::SourceFileIo(source_path.clone(), e))?;

    let size = Some(metadata.len());
    let source = Source {
      sha256,
      size,
      label: None,
    };
    Ok(source)
  }

  pub(crate) fn compute_sha256(source_path: &PathBuf) -> Result<String, AppError> {
    let file =
      File::open(source_path).map_err(|e| AppError::SourceFileIo(source_path.clone(), e))?;

    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
      let n = reader
        .read(&mut buffer)
        .map_err(|e| AppError::SourceFileIo(source_path.clone(), e))?;
      if n == 0 {
        break;
      }
      hasher.update(&buffer[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    Ok(actual)
  }
}
