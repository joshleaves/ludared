use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::virtual_path::complete_virtual_path;
use crate::errors::app_error::AppError;
use crate::project::Project;

#[derive(Args)]
pub(crate) struct CacheListArgs {
  /// Virtual path to render as the root of the tree
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_virtual_path))]
  virtual_path: Option<String>,
}

pub(crate) fn command_cache_list(project: &Project, args: &CacheListArgs) -> Result<(), AppError> {
  info!("Listing decode cache for project '{}'", project.name());

  project
    .cache
    .entries()
    .filter(|vpath| {
      args
        .virtual_path
        .as_ref()
        .is_none_or(|prefix| vpath == prefix || vpath.starts_with(&format!("{prefix}/")))
    })
    .for_each(|vpath| println!("{vpath}"));

  // for virtual_path in project.cache.entries() {
  //   println!("{virtual_path}");
  // }

  Ok(())
}
