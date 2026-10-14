use std::path::PathBuf;

use crate::errors::app_error::AppError;
use crate::project::Project;
use clap::Args;

#[derive(Args)]
pub(crate) struct CodecsDetectArgs {
  /// File
  #[arg(value_hint = clap::ValueHint::FilePath)]
  file: PathBuf,
}

pub(crate) fn command_codecs_detect(args: &CodecsDetectArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;
  let data = std::fs::read(&args.file)?;
  for (codec, confidence) in project.codecs.detect(&data)? {
    println!("{:>8}: {}", confidence, codec.id());
  }

  Ok(())
}
