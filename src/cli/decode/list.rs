use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::artifacts::completion_artifacts_decoded;
use ludared::errors::app_error::AppError;
use ludared::project::Project;
use ludared::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct DecodeListArgs {
  /// Virtual path of the artifact whose decodes to list
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(completion_artifacts_decoded))]
  virtual_path: String,
}

pub(crate) fn command_decode_list(args: &DecodeListArgs) -> Result<(), AppError> {
  info!("Listing decodes for '{}'", args.virtual_path);

  let vpath = VirtualPath::new(&args.virtual_path)?;
  let project = Project::load_default()?;

  let decodes = project.manifest.decodes_at(&vpath)?;

  let width = decodes
    .iter()
    .map(|decode| decode.name.len())
    .max()
    .unwrap_or(0);

  for decode in decodes {
    println!("{:<width$} {}", decode.name, decode.codec.id);
  }

  Ok(())
}
