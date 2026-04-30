use log::*;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::PathBuf;

use crate::{app_error::AppError, manifest::Source, project::Project};

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

  // project.verify_sources()?;
  println!("✓ All sources OK");

  println!("✓ Doctor check passed");
  Ok(())
}

fn verify_source(source_path: PathBuf, source: &Source) -> Result<(), AppError> {
  let file =
    File::open(&source_path).map_err(|e| AppError::SourceFileIo(source_path.clone(), e))?;

  let mut reader = BufReader::new(file);
  let mut hasher = Sha256::new();
  let mut buffer = [0u8; 8192];

  loop {
    let n = reader
      .read(&mut buffer)
      .map_err(|e| AppError::SourceFileIo(source_path.clone(), e))?;
    if n == 0 {
      break;
    }
    hasher.update(&buffer[..n]);
  }
  let actual = hex::encode(hasher.finalize());

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
