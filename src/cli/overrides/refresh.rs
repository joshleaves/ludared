use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::overrides::completion_override_active;
use crate::cli::path_glob::PathGlob;
use ludared::errors::app_error::AppError;
use ludared::project::Project;
use ludared::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct OverrideRefreshArgs {
  /// Virtual path of the overridden artifact to reset
  ///
  /// This rewrites the workspace file from the decode cache, discarding every
  /// local modification to it and recreating it if it was deleted. Nothing is
  /// asked for confirmation.
  #[arg(value_name = "PATH*", add = ArgValueCompleter::new(completion_override_active))]
  virtual_path: String,
}

pub(crate) fn command_override_refresh(
  project: &mut Project,
  args: &OverrideRefreshArgs,
) -> Result<(), AppError> {
  let glob = PathGlob::new(&args.virtual_path);

  let matches: Vec<_> = if glob.is_glob() {
    project
      .manifest
      .overrides()
      .into_iter()
      .filter(|path| glob.matches(path.as_str()))
      .collect()
  } else {
    vec![VirtualPath::new(
      &args.virtual_path,
    )?]
  };

  if matches.is_empty() {
    return Err(AppError::NoGlobMatches(args.virtual_path.clone()));
  }

  for vpath in matches {
    project.refresh_override(&vpath)?;
    println!("✓ Refreshed '{vpath}' from its canonical bytes");
  }

  Ok(())
}
