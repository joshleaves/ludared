use crate::{app_error::AppError, project::Project};
use log::*;

pub(crate) fn command_doctor() -> Result<(), AppError> {
  let project = Project::load_default()?;
  info!(
    "Verifying sources for project {}",
    project.configuration.project.name
  );
  project.verify_sources()?;
  println!("✓ All sources OK");

  println!("✓ Doctor check passed");
  Ok(())
}
