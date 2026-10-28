use crate::errors::app_error::AppError;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize)]
pub struct Source {
  pub sha256: String,
  #[serde(default)]
  pub size: Option<u64>,
  #[serde(default)]
  pub label: Option<String>,
}

impl Source {
  pub fn new_from_file(source_path: &PathBuf) -> Result<Self, AppError> {
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

  /// Computes a file's SHA-256 without loading it entirely into memory.
  /// Used when only the checksum is needed, such as during `doctor`.
  pub fn compute_sha256(source_path: &PathBuf) -> Result<String, AppError> {
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

  /// Verifies the SHA-256 of bytes already loaded into memory.
  /// Used by `build` and `unpack`, which need the bytes for decoding anyway.
  pub fn sha256_matches(&self, bytes: &[u8]) -> Result<(), String> {
    let actual = hex::encode(Sha256::digest(bytes));

    if actual == self.sha256 {
      Ok(())
    } else {
      Err(actual)
    }
  }
}
