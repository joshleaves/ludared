use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::artifacts::completion_artifacts_decoded;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct DecodeRemoveArgs {
  /// Virtual path of the artifact whose decode to remove
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(completion_artifacts_decoded))]
  virtual_path: String,

  /// Name of the decode to remove
  ///
  /// Only needed when several decodes are attached to the same artifact.
  // TODO: Once we can access the previous state of the current command,
  // we can play with that properly.
  #[arg(value_name = "NAME")]
  name: Option<String>,

  /// Delete the workspace files of the overrides the decode declared
  ///
  /// Only those single files are removed, never a parent directory or any other
  /// file of the workspace. Without it, the files are kept as they are.
  #[arg(long)]
  clean: bool,
}

pub(crate) fn command_decode_remove(args: &DecodeRemoveArgs) -> Result<(), AppError> {
  let vpath = VirtualPath::new(&args.virtual_path)?;
  let mut project = Project::load_default()?;
  let removed = project.remove_decode(&vpath, args.name.as_deref(), args.clean)?;

  println!("✓ Removed decode '{}' from {vpath}", removed.name);

  Ok(())
}
