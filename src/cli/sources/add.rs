use crate::cli::completions::sources::complete_source_add;
use crate::{app_error::AppError, project::Project};
use clap::Args;
use clap_complete::engine::ArgValueCompleter;
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct SourcesAddArgs {
  /// Source file to add
  #[arg(add = ArgValueCompleter::new(complete_source_add))]
  file: PathBuf,
}

/// Adds a source file to the project and reports success to the user.
pub(crate) fn command_sources_add(
  project: &mut Project,
  args: &SourcesAddArgs,
) -> Result<(), AppError> {
  project.add_source(&args.file)?;

  println!("✓ Added source {}", args.file.display());
  Ok(())
}
