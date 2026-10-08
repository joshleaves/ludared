use log::*;

use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::project::cache::errors::CacheError;
use crate::virtual_path::VirtualPath;
use clap::Args;

#[derive(Args)]
pub(crate) struct CachePathArgs {
  /// Virtual path
  virtual_path: String,
}

pub(crate) fn command_cache_path(project: &Project, args: &CachePathArgs) -> Result<(), AppError> {
  let vpath = VirtualPath::new(&args.virtual_path)?;
  info!(
    "Locating '{vpath}' in the decode cache of '{}'",
    project.name()
  );

  let blob = project
    .cache
    .get_entry(&vpath)
    .ok_or_else(|| CacheError::NotIndexed(vpath.to_string()))?;

  println!("{}", blob.display());

  Ok(())
}
