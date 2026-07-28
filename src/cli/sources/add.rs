use crate::{app_error::AppError, project::Project, source::Source};
use clap::Args;
use log::*;
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct SourcesAddArgs {
  /// Source file to add
  file: PathBuf,
}

pub(crate) fn command_sources_add(args: &SourcesAddArgs) -> Result<(), AppError> {
  let mut project = Project::load_default()?;
  let source_path = project.source_path(&args.file);
  if !source_path.is_file() {
    warn!("File missing: {}", source_path.display());
    return Err(AppError::NoSuchFile(args.file.clone()));
  };

  if project.manifest.sources.contains_key(&args.file) {
    warn!("File already in manifest: {}", &args.file.display());
    return Err(AppError::SourceFileAlreadyExists(args.file.clone()));
  }
  let new_source = Source::new_from_file(&source_path)?;
  project
    .manifest
    .sources
    .insert(args.file.clone(), new_source);
  project.save_manifest()?;

  println!("✓ Added source {}", &args.file.display());

  Ok(())
}
