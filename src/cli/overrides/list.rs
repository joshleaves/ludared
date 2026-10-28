use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::overrides::completion_override_list;
use crate::cli::listing;
use ludared::errors::app_error::AppError;
use ludared::project::Project;
use ludared::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct OverrideListArgs {
  /// Virtual path to filter the overrides by
  ///
  /// The path is matched against whole components, so a parent that overrides sit
  /// under filters them all, whether or not it is an artifact of its own.
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(completion_override_list))]
  virtual_path: Option<String>,

  /// Display the overrides as a tree
  #[arg(long)]
  tree: bool,
}

pub(crate) fn command_override_list(
  project: &Project,
  args: &OverrideListArgs,
) -> Result<(), AppError> {
  info!("Listing overrides for project '{}'", project.name());

  let root = match &args.virtual_path {
    Some(path) => Some(VirtualPath::new(path)?),
    None => None,
  };

  listing::print(&project.manifest.overrides(), root.as_ref(), args.tree);

  Ok(())
}
