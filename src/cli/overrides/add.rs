use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::overrides::complete_override_addable;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct OverrideAddArgs {
  /// Virtual path of the decoded artifact to override
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_override_addable))]
  virtual_path: String,

  /// Replace a workspace file that is not the output of an active override
  #[arg(long)]
  force: bool,
}

pub(crate) fn command_override_add(
  project: &mut Project,
  args: &OverrideAddArgs,
) -> Result<(), AppError> {
  let vpath = VirtualPath::new(&args.virtual_path)?;

  project.add_override(&vpath, args.force)?;

  println!("✓ Overrode '{vpath}' in the workspace");

  Ok(())
}
