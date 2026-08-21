use crate::errors::app_error::AppError;
use crate::hash::sha256_bytes;
use crate::virtual_path::VirtualPath;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::create_dir_all;
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct CacheIndex {
  entries: BTreeMap<String, String>,
}

#[derive(Debug)]
pub(crate) struct Cache {
  pub(crate) root: PathBuf,
  pub(crate) index: CacheIndex,
}

impl Cache {
  pub fn new(root: PathBuf) -> Result<Self, AppError> {
    create_dir_all(&root)?;
    let index = match std::fs::read(root.join("index.json")) {
      Ok(data) => serde_json::from_slice(&data).map_err(AppError::CacheIndexJson)?,
      Err(err) => match err.kind() {
        std::io::ErrorKind::NotFound => CacheIndex::default(),
        _ => return Err(err.into()),
      },
    };

    Ok(Self { root, index })
  }

  pub fn entries(&self) -> impl Iterator<Item = &str> {
    self.index.entries.keys().map(String::as_str)
  }

  pub fn add_entry(&mut self, path: VirtualPath, bytes: &[u8]) -> Result<(), AppError> {
    let hash = sha256_bytes(bytes);
    std::fs::write(self.root.join(&hash), bytes)?;
    self.index.entries.insert(path.to_string(), hash);
    Ok(())
  }

  pub fn get_entry(&self, path: &VirtualPath) -> Option<PathBuf> {
    Some(self.root.join(self.index.entries.get(&path.to_string())?))
  }

  // pub fn remove_entry(&mut self, path: &VirtualPath) -> Result<(), AppError> {
  //   self.index.entries.remove_entry(&path.to_string());
  //   Ok(())
  // }

  pub fn save(&self) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(&self.index).map_err(AppError::CacheIndexJson)?;
    std::fs::write(self.root.join("index.json"), json).map_err(AppError::CacheIndexIo)?;
    Ok(())
  }
}
