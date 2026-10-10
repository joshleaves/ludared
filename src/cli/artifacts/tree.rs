use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::artifacts::complete_artifacts_listing;
use crate::cli::tree_formatter::render;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct ArtifactsTreeArgs {
  /// Virtual path to render as the root of the tree
  ///
  /// The path is matched against whole components, so a parent that artifacts sit
  /// under renders them all, whether or not it is an artifact of its own.
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_artifacts_listing))]
  virtual_path: Option<String>,
}

pub(crate) fn command_artifacts_tree(
  project: &Project,
  args: &ArtifactsTreeArgs,
) -> Result<(), AppError> {
  let root = match &args.virtual_path {
    Some(path) => {
      let root = VirtualPath::new(path)?;
      info!("Rendering '{root}' as an artifact tree");
      Some(root)
    }
    None => {
      info!("Rendering the artifacts as a tree");
      None
    }
  };

  let entries: Vec<String> = project
    .manifest
    .artifacts()
    .iter()
    .map(VirtualPath::to_string)
    .collect();
  let entries: Vec<&str> = entries.iter().map(String::as_str).collect();

  // The renderer scopes the tree to the root it is given.
  print!("{}", render(&entries, root.as_ref()));

  Ok(())
}
