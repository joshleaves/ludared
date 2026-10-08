use std::fs::File;
use std::io::Write as _;

use clap::Args;
use log::*;

use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::project::cache::errors::CacheError;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct CacheCatArgs {
  /// Virtual path
  virtual_path: String,
}

pub(crate) fn command_cache_cat(project: &Project, args: &CacheCatArgs) -> Result<(), AppError> {
  let vpath = VirtualPath::new(&args.virtual_path)?;
  info!(
    "Reading '{vpath}' from the decode cache of '{}'",
    project.name()
  );

  let blob = project
    .cache
    .get_entry(&vpath)
    .ok_or_else(|| CacheError::NotIndexed(vpath.to_string()))?;

  // Artifacts are binary, and can be large, so the blob is streamed out verbatim
  // rather than through a formatting helper that would expect text.
  let mut blob = File::open(blob)?;
  let mut stdout = std::io::stdout().lock();
  std::io::copy(&mut blob, &mut stdout)?;
  stdout.flush()?;

  Ok(())
}
