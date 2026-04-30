use log::*;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use crate::app_error::AppError;
use crate::configuration::Configuration;
use crate::manifest::{Manifest, Source};

#[derive(Debug)]
pub(crate) struct Project {
  pub(crate) root: PathBuf,
  pub(crate) configuration: Configuration,
  pub(crate) manifest: Manifest,
}

impl Project {
  pub(crate) fn load(root: &Path) -> Result<Self, AppError> {
    let configuration_path = root.join("ludared.toml");
    let configuration = Configuration::load(&configuration_path)?;
    debug!("Parsed configuration: ludared.toml");
    trace!("{configuration:?}");

    let manifest_path = root.join(&configuration.project.manifest);
    let manifest = Manifest::load(&manifest_path)?;
    debug!(
      "Parsed manifest: {}",
      &configuration.project.manifest.display()
    );
    trace!("{manifest:?}");

    Ok(Self {
      root: root.to_path_buf(),
      configuration,
      manifest,
    })
  }

  pub(crate) fn load_default() -> Result<Self, AppError> {
    Self::load(Path::new("."))
  }

  pub(crate) fn sources_path(&self) -> PathBuf {
    self.root.join(&self.configuration.paths.sources)
  }

  pub(crate) fn source_path(&self, source_name: &Path) -> PathBuf {
    self.sources_path().join(source_name)
  }

  pub(crate) fn verify_sources(&self) -> Result<(), AppError> {
    if self.manifest.sources.is_empty() {
      warn!("No source files defined in project");
      return Ok(());
    }

    for (source_name, source) in self.manifest.sources.iter() {
      debug!("Verifying source file: {}", source_name.display());
      self.verify_source(source_name, source)?;
    }
    Ok(())
  }

  pub(crate) fn verify_source(&self, source_name: &Path, source: &Source) -> Result<(), AppError> {
    let source_path = self.source_path(source_name);
    let file =
      File::open(&source_path).map_err(|e| AppError::SourceFileIo(source_path.clone(), e))?;

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

    if actual != source.sha256 {
      return Err(AppError::SourceFileMismatch(
        source_path.clone(),
        source.sha256.clone(),
        actual,
      ));
    }
    debug!("Matching SHA256: {}", actual);

    Ok(())
  }
}
