use log::*;
use std::path::PathBuf;

use crate::{errors::app_error::AppError, project::Project, source::Source};

pub(crate) fn command_doctor() -> Result<(), AppError> {
  let project = Project::load_default()?;
  info!("Verifying sources for project {}", project.name());
  if project.manifest.sources.is_empty() {
    warn!("No source files defined in project");
    return Ok(());
  }
  let sources_path = project.root.join(project.configuration.paths.sources);
  for (source_name, source) in project.manifest.sources.iter() {
    debug!("Verifying source file: {}", source_name.display());
    verify_source(sources_path.join(source_name), source)?;
  }

  // TODO: Later separate into `project.verify_sources()?;`
  println!("✓ All sources OK");

  println!("✓ Doctor check passed");
  Ok(())
}

fn verify_source(source_path: PathBuf, source: &Source) -> Result<(), AppError> {
  let actual = Source::compute_sha256(&source_path)?;
  if actual != source.sha256 {
    return Err(AppError::SourceFileMismatch(
      source_path.clone(),
      source.sha256.clone(),
      actual,
    ));
  }
  debug!("Matching SHA256: {}", actual);

  Ok(())
}
