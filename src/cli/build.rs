use clap::Args;
use clap_complete::engine::ArgValueCompleter;
use log::*;
use std::path::Path;
use std::path::PathBuf;

use crate::cli::completions::sources::completion_sources_list;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;
use crate::virtual_path::errors::VirtualPathError;

#[derive(Args)]
pub(crate) struct BuildArgs {
  /// Source to build; every registered source when omitted
  #[arg(value_name = "SOURCE", add = ArgValueCompleter::new(completion_sources_list))]
  source: Option<String>,
}

/// Rebuilds a source from its decode tree and writes it under the builds root.
pub(crate) fn command_build(args: &BuildArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;
  info!("Building project '{}'", project.name());

  for source in select_sources(&project, args.source.as_deref())? {
    let output = output_path(&project, &source)?;
    let bytes = project.build(&source)?;

    debug!("Writing {} bytes to {}", bytes.len(), output.display());
    if let Some(parent) = output.parent() {
      std::fs::create_dir_all(parent)
        .map_err(|err| AppError::BuildIo(parent.to_path_buf(), err))?;
    }
    std::fs::write(&output, &bytes).map_err(|err| AppError::BuildIo(output.clone(), err))?;

    println!("✓ Built {}", output.display());
  }

  Ok(())
}

fn select_sources(
  project: &Project,
  requested: Option<&str>,
) -> Result<Vec<VirtualPath>, AppError> {
  let mut sources: Vec<&PathBuf> = project.manifest.sources.keys().collect();
  sources.sort();

  match requested {
    Some(name) if project.manifest.sources.contains_key(Path::new(name)) => {
      Ok(vec![virtual_path_of(
        Path::new(name),
      )?])
    }
    Some(name) => Err(AppError::SourceNotFound(PathBuf::from(name))),
    None if sources.is_empty() => Err(AppError::NoBuildSource),
    None => sources
      .into_iter()
      .map(|source| virtual_path_of(source))
      .collect(),
  }
}

fn output_path(project: &Project, source: &VirtualPath) -> Result<PathBuf, AppError> {
  let name = source.to_string();
  Ok(Path::new(&project.configuration.paths.builds).join(&name))
}

fn virtual_path_of(name: &Path) -> Result<VirtualPath, AppError> {
  let name = name
    .to_str()
    .ok_or_else(|| VirtualPathError::NonUtf8Path(name.to_path_buf()))?;

  VirtualPath::new(name).map_err(AppError::from)
}
