use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::artifacts::complete_artifacts_listing;
use crate::cli::listing;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct ArtifactsListArgs {
  /// Virtual path to filter the artifacts by
  ///
  /// The path is matched against whole components, so a parent that artifacts sit
  /// under filters them all, whether or not it is an artifact of its own.
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_artifacts_listing))]
  virtual_path: Option<String>,

  /// Display the artifacts as a tree
  #[arg(long)]
  tree: bool,
}

pub(crate) fn command_artifacts_list(
  project: &Project,
  args: &ArtifactsListArgs,
) -> Result<(), AppError> {
  info!("Listing artifacts of project '{}'", project.name());

  let root = match &args.virtual_path {
    Some(path) => Some(VirtualPath::new(path)?),
    None => None,
  };

  listing::print(&project.manifest.artifacts(), root.as_ref(), args.tree);

  Ok(())
}
