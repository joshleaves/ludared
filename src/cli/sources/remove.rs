use crate::{app_error::AppError, project::Project};
use clap::Args;
use log::*;
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct SourcesRemoveArgs {
  /// Source file to remove
  file: PathBuf,
}

pub(crate) fn command_sources_remove(args: &SourcesRemoveArgs) -> Result<(), AppError> {
  let mut project = Project::load_default()?;
  if project.manifest.sources.remove(&args.file).is_none() {
    warn!("Source not found in manifest: {}", &args.file.display());
    return Err(AppError::SourceNotFound(args.file.clone()));
  };
  project.save_manifest()?;
  println!("✓ Removed source {}", &args.file.display());

  Ok(())
}
