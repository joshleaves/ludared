use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::overrides::complete_override_active;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::project::overrides::errors::OverrideError;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct OverrideRemoveArgs {
  /// Virtual path of the artifact to stop overriding
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_override_active))]
  virtual_path: String,

  /// Delete the workspace file backing the artifact
  ///
  /// Only that single file is removed, never a parent directory or any other
  /// file of the workspace. Without it, the file is kept as it is.
  #[arg(long)]
  clean: bool,
}

pub(crate) fn command_override_remove(
  project: &mut Project,
  args: &OverrideRemoveArgs,
) -> Result<(), AppError> {
  let vpath = VirtualPath::new(&args.virtual_path)?;

  // The file goes first, while the manifest still declares the override: a
  // deletion that fails then leaves the artifact overridden rather than dropping
  // the marker of a workspace file that is still there.
  if args.clean {
    if !project.manifest.is_overridden(&vpath) {
      return Err(OverrideError::NotOverridden(vpath.to_string()).into());
    }

    // The same mapping every other override command writes through, so a path that
    // names no virtual path cannot escape the workspace root.
    let destination = project.workspace_path(&vpath);

    match std::fs::remove_file(&destination) {
      Ok(()) => debug!("Deleted the workspace file of '{vpath}'"),
      Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
        trace!("Nothing to clean for '{vpath}', its workspace file is already gone");
      }
      Err(err) => return Err(AppError::WorkspaceFileIo(destination, err)),
    }
  }

  project.remove_override(&vpath)?;

  match args.clean {
    true => println!("✓ Removed the override of '{vpath}' and its workspace file"),
    false => println!("✓ Removed the override of '{vpath}', keeping its workspace file"),
  }

  Ok(())
}
