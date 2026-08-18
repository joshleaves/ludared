use log::*;
use std::path::{Path, PathBuf};

use crate::configuration::Configuration;
use crate::errors::app_error::AppError;
use crate::manifest::Manifest;
use crate::project::cache::Cache;

pub(crate) mod cache;
pub(crate) mod sources;

#[derive(Debug)]
pub(crate) struct Project {
  pub(crate) root: PathBuf,
  pub(crate) configuration: Configuration,
  pub(crate) manifest: Manifest,
  pub(crate) cache: Cache,
}

impl Project {
  pub(crate) fn load(root: &Path) -> Result<Self, AppError> {
    trace!("Loading up project configuration");
    let configuration_path = root.join("ludared.toml");
    let configuration = Configuration::load(&configuration_path)?;
    debug!("Parsed configuration: ludared.toml");
    trace!("{configuration:?}");

    trace!("Loading up project manifest");
    let manifest_path = root.join(&configuration.project.manifest);
    let manifest = Manifest::load(&manifest_path)?;
    debug!(
      "Parsed manifest: {}",
      &configuration.project.manifest.display()
    );
    trace!("{manifest:?}");

    let cache = Cache::new(configuration.paths.cache.join("decodes"))?;

    Ok(Self {
      root: root.to_path_buf(),
      configuration,
      manifest,
      cache,
    })
  }

  pub(crate) fn load_default() -> Result<Self, AppError> {
    Self::load(Path::new("."))
  }

  pub fn name(&self) -> &String {
    &self.configuration.project.name
  }

  pub fn save_manifest(&self) -> Result<(), AppError> {
    trace!("Saving project manifest");
    let manifest_path = self.root.join(&self.configuration.project.manifest);
    self.manifest.save(&manifest_path)?;

    debug!(
      "Saved manifest: {}",
      &self.configuration.project.manifest.display()
    );
    trace!("{:?}", self.manifest);
    Ok(())
  }
}
