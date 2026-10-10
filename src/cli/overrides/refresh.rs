use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::overrides::complete_override_active;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct OverrideRefreshArgs {
  /// Virtual path of the overridden artifact to reset
  ///
  /// This rewrites the workspace file from the decode cache, discarding every
  /// local modification to it and recreating it if it was deleted. Nothing is
  /// asked for confirmation.
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_override_active))]
  virtual_path: String,
}

pub(crate) fn command_override_refresh(
  project: &mut Project,
  args: &OverrideRefreshArgs,
) -> Result<(), AppError> {
  let vpath = VirtualPath::new(&args.virtual_path)?;

  project.refresh_override(&vpath)?;

  println!("✓ Refreshed '{vpath}' from its canonical bytes");

  Ok(())
}
