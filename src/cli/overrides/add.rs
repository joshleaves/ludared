use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::overrides::completion_override_addable;
use crate::cli::path_glob::PathGlob;
use ludared::errors::app_error::AppError;
use ludared::project::Project;
use ludared::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct OverrideAddArgs {
  /// Virtual path of the decoded artifact to override
  #[arg(value_name = "PATH*", add = ArgValueCompleter::new(completion_override_addable))]
  virtual_path: String,

  /// Replace a workspace file that is not the output of an active override
  #[arg(long)]
  force: bool,
}

pub(crate) fn command_override_add(
  project: &mut Project,
  args: &OverrideAddArgs,
) -> Result<(), AppError> {
  let glob = PathGlob::new(&args.virtual_path);
  let matches: Vec<_> = if glob.is_glob() {
    project
      .manifest
      .artifacts()
      .into_iter()
      .filter(|path| !path.is_source())
      .filter(|path| project.manifest.is_terminal(path))
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
    project.add_override(&vpath, args.force)?;

    println!("✓ Overrode '{vpath}' in the workspace");
  }

  Ok(())
}
