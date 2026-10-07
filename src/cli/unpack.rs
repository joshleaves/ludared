use log::*;

use crate::errors::app_error::AppError;
use crate::project::Project;

pub(crate) fn command_unpack() -> Result<(), AppError> {
  let mut project = Project::load_default()?;
  info!("Unpacking project '{}'", project.name());

  let report = project.unpack()?;

  println!(
    "✓ Restored {} artifact(s) from {} decode(s)",
    report.artifacts, report.decodes
  );

  Ok(())
}
