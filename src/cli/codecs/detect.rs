use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::artifacts::completion_artifacts_list;
use crate::cli::path_glob::PathGlob;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::project::cache::errors::CacheError;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct CodecsDetectArgs {
  /// File
  #[arg(value_name = "PATH*", add = ArgValueCompleter::new(completion_artifacts_list))]
  virtual_path: String,
}

pub(crate) fn command_codecs_detect(args: &CodecsDetectArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;
  let glob = PathGlob::new(&args.virtual_path);
  let matches: Vec<_> = if glob.is_glob() {
    project
      .manifest
      .artifacts()
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
    let cache_path = project
      .cache
      .get_entry(&vpath)
      .ok_or_else(|| CacheError::NotIndexed(vpath.to_string()))?;
    let data = std::fs::read(cache_path)?;
    for (codec, confidence) in project.codecs.detect(&data)? {
      println!("{}\t{:>8}: {}", vpath, confidence, codec.id());
    }
  }

  Ok(())
}
