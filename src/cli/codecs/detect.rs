use std::path::PathBuf;

use crate::codecs::registry::CodecRegistry;
use crate::errors::app_error::AppError;
use clap::Args;

#[derive(Args)]
pub(crate) struct CodecsDetectArgs {
  /// File
  #[arg(value_hint = clap::ValueHint::FilePath)]
  file: PathBuf,
}

pub(crate) fn command_codecs_detect(args: &CodecsDetectArgs) -> Result<(), AppError> {
  let data = std::fs::read(&args.file)?;
  for (codec, confidence) in CodecRegistry::detect(&data) {
    println!("{:>8}: {}", confidence.to_string(), codec.id());
  }

  Ok(())
}
