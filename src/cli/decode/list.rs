use log::info;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::artifacts::completion_artifacts_decoded;
use crate::cli::path_glob::PathGlob;
use ludared::errors::app_error::AppError;
use ludared::project::Project;
use ludared::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct DecodeListArgs {
  /// Virtual path or glob of artifacts whose decodes to list (defaults to all)
  #[arg(value_name = "PATH*", add = ArgValueCompleter::new(completion_artifacts_decoded))]
  virtual_path: Option<String>,
}

pub(crate) fn command_decode_list(args: &DecodeListArgs) -> Result<(), AppError> {
  let pattern = args.virtual_path.as_deref().unwrap_or("**");
  info!("Listing decodes matching '{pattern}'");

  let project = Project::load_default()?;
  let glob = PathGlob::new(pattern);
  let include_path = args.virtual_path.is_none() || glob.is_glob();
  let matches: Vec<_> = if glob.is_glob() {
    project
      .manifest
      .artifacts()
      .into_iter()
      .filter(|path| glob.matches(path.as_str()))
      .collect()
  } else {
    vec![VirtualPath::new(
      pattern,
    )?]
  };

  if args.virtual_path.is_some() && glob.is_glob() && matches.is_empty() {
    return Err(AppError::NoGlobMatches(pattern.to_owned()));
  }

  for path in matches {
    let decodes = project.manifest.decodes_at(&path)?;
    let name_width = decodes
      .iter()
      .map(|decode| decode.name.len())
      .max()
      .unwrap_or(0);

    for decode in decodes {
      if include_path {
        print!("{path}\t");
      }

      println!("{:<name_width$} {}", decode.name, decode.codec.id);
    }
  }

  Ok(())
}
