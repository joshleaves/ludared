use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::overrides::complete_override_listing;
use crate::cli::tree_formatter::render;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct OverrideTreeArgs {
  /// Virtual path to render as the root of the tree
  ///
  /// The path is matched against whole components, so a parent that overrides sit
  /// under renders them all, whether or not it is an artifact of its own.
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_override_listing))]
  virtual_path: Option<String>,
}

pub(crate) fn command_override_tree(
  project: &Project,
  args: &OverrideTreeArgs,
) -> Result<(), AppError> {
  let root = match &args.virtual_path {
    Some(path) => Some(VirtualPath::new(path)?),
    None => None,
  };

  match root.as_ref() {
    Some(root) => info!("Rendering the overrides under '{root}' as a tree"),
    None => info!("Rendering the overrides as a tree"),
  }

  let entries: Vec<String> = project
    .manifest
    .overrides()
    .iter()
    .map(VirtualPath::to_string)
    .collect();
  let entries: Vec<&str> = entries.iter().map(String::as_str).collect();

  // The renderer scopes the tree to the root it is given, and draws the implicit
  // parents the overrides hang under.
  print!("{}", render(&entries, root.as_ref()));

  Ok(())
}
