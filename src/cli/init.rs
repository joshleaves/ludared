use crate::configuration::Configuration;
use crate::configuration::PathsConfiguration;
use crate::configuration::ProjectConfiguration;
use crate::errors::app_error::AppError;
use crate::manifest::Manifest;
use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct InitArgs {
  /// Project name
  #[arg(long)]
  pub name: Option<String>,

  /// Path to the project manifest
  #[arg(long)]
  pub manifest: Option<PathBuf>,

  /// Directory containing source files
  #[arg(long)]
  pub sources: Option<PathBuf>,

  /// Directory for build outputs
  #[arg(long)]
  pub builds: Option<PathBuf>,

  /// Directory for cache files
  #[arg(long)]
  pub cache: Option<PathBuf>,

  /// Use default values without prompting
  #[arg(long)]
  pub non_interactive: bool,
  // /// Overwrite existing configuration files
  // #[arg(short, long)]
  // pub force: bool,
}

impl InitArgs {
  pub(crate) fn name_value(&self) -> String {
    if let Some(name) = &self.name {
      return name.clone();
    }
    std::env::current_dir()
      .unwrap_or_else(|_| PathBuf::from("project"))
      .file_name()
      .and_then(|s| s.to_str())
      .unwrap_or("project")
      .to_owned()
  }

  pub(crate) fn manifest_value(&self) -> PathBuf {
    if let Some(manifest) = &self.manifest {
      return manifest.clone();
    }
    PathBuf::from(format!("{}.ludared", self.name_value()))
  }

  pub(crate) fn sources_value(&self) -> PathBuf {
    if let Some(sources) = &self.sources {
      return sources.clone();
    }
    PathBuf::from("sources")
  }

  pub(crate) fn builds_value(&self) -> PathBuf {
    if let Some(builds) = &self.builds {
      return builds.clone();
    }
    PathBuf::from("builds")
  }

  pub(crate) fn cache_value(&self) -> PathBuf {
    if let Some(cache) = &self.cache {
      return cache.clone();
    }
    self.builds_value().join("cache")
  }

  pub(crate) fn initialize_project(&self) -> Result<(), AppError> {
    let project = ProjectConfiguration {
      name: self.name_value(),
      manifest: self.manifest_value(),
    };
    let paths = PathsConfiguration {
      sources: self.sources_value(),
      builds: self.builds_value(),
      cache: self.cache_value(),
    };
    let configuration = Configuration { project, paths };
    configuration.save(&PathBuf::from("ludared.toml"))?;
    std::fs::create_dir_all(self.sources_value())?;
    std::fs::create_dir_all(self.builds_value())?;
    std::fs::create_dir_all(self.cache_value())?;
    Manifest::default().save(&self.manifest_value())?;

    Ok(())
  }
}

pub(crate) fn command_init(args: &mut InitArgs) -> Result<(), AppError> {
  if PathBuf::from("ludared.toml").exists() {
    return Err(AppError::ConfigurationFileAlreadyExists());
  }

  if !args.non_interactive {
    if args.name.is_none() {
      args.name = Some(prompt("Project name", &args.name_value())?);
    }

    let default = args.manifest_value();
    prompt_path(&mut args.manifest, "Project manifest", default)?;

    let default = args.sources_value();
    prompt_path(&mut args.sources, "Sources directory", default)?;

    let default = args.builds_value();
    prompt_path(&mut args.builds, "Builds directory", default)?;

    let default = args.cache_value();
    prompt_path(&mut args.cache, "Cache directory", default)?;
  }

  args.initialize_project()
}

use std::io::{self, Write};

fn prompt(label: &str, default: &str) -> Result<String, AppError> {
  print!("{} [{}]: ", label, default);
  io::stdout()
    .flush()
    .map_err(AppError::ConfigurationFileIo)?;

  let mut value = String::new();
  io::stdin()
    .read_line(&mut value)
    .map_err(AppError::ConfigurationFileIo)?;

  let value = value.trim();

  Ok(if value.is_empty() {
    default.to_owned()
  } else {
    value.to_owned()
  })
}

fn prompt_path(value: &mut Option<PathBuf>, label: &str, default: PathBuf) -> Result<(), AppError> {
  if value.is_none() {
    *value = Some(PathBuf::from(prompt(
      label,
      &default.display().to_string(),
    )?));
  }

  Ok(())
}
