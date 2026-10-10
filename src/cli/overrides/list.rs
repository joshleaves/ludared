use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::overrides::complete_override_listing;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct OverrideListArgs {
  /// Virtual path to filter the overrides by
  ///
  /// The path is matched against whole components, so a parent that overrides sit
  /// under filters them all, whether or not it is an artifact of its own.
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_override_listing))]
  virtual_path: Option<String>,
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

  for vpath in project
    .manifest
    .overrides()
    .iter()
    .filter(|vpath| root.as_ref().is_none_or(|root| vpath.is_within(root)))
  {
    println!("{vpath}");
  }

  Ok(())
}
