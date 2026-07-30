use crate::{app_error::AppError, project::Project};
use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct SourcesRemoveArgs {
  /// Source file to remove
  file: PathBuf,
}

/// Removes a source file from the project and reports success to the user.
pub(crate) fn command_sources_remove(
  project: &mut Project,
  args: &SourcesRemoveArgs,
) -> Result<(), AppError> {
  project.remove_source(&args.file)?;

  println!("✓ Removed source {}", args.file.display());
  Ok(())
}
