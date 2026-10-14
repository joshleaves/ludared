use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::artifacts::complete_artifacts_listing;

use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::project::cache::errors::CacheError;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct CachePathArgs {
  /// Virtual path
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_artifacts_listing))]
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
