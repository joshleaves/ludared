use crate::{app_error::AppError, configuration::Configuration};
use clap::Args;
use log::*;
use std::path::{Path, PathBuf};

#[derive(Args)]
pub(crate) struct CleanArgs {
  #[arg(long)]
  dry_run: bool,
}

pub(crate) fn command_clean(args: &CleanArgs) -> Result<(), AppError> {
  trace!("Loading up project configuration");
  let configuration = Configuration::load_default()?;
  let root = Path::new(".");
  let builds = root.join(&configuration.paths.builds);
  let cache = root.join(&configuration.paths.cache);
  let dry_run = args.dry_run;

  let clean_targets = determine_targets(&builds, &cache);
  if clean_targets.is_empty() {
    success_message(dry_run);
    return Ok(());
  }

  match clean_targets.as_slice() {
    [("builds", _)] => debug!("Clean targets: builds"),
    [("cache", _)] => debug!("Clean targets: cache"),
    [
      ("builds", _),
      ("cache", _),
    ] => debug!("Clean targets: builds, cache"),
    _ => debug!("Clean targets: <unknown>"),
  }

  for (_, target) in clean_targets {
    match dry_run {
      true => clean_path_dry(target)?,
      false => clean_path(target)?,
    }
  }

  success_message(dry_run);
  Ok(())
}

fn success_message(dry_run: bool) {
  match dry_run {
    true => println!("✓ Dry run complete"),
    false => println!("✓ All artifacts cleaned"),
  }
}

fn determine_targets(builds: &Path, cache: &Path) -> Vec<(&'static str, PathBuf)> {
  let mut targets = Vec::new();

  if builds.is_dir() {
    targets.push(("builds", builds.to_path_buf()));

    let builds_abs = builds.canonicalize().unwrap_or(builds.to_path_buf());
    let cache_abs = cache.canonicalize().unwrap_or(cache.to_path_buf());

    if cache_abs.starts_with(&builds_abs) {
      return targets;
    }
  }

  if cache.is_dir() {
    targets.push(("cache", cache.to_path_buf()));
  }

  targets
}

fn clean_path(path: PathBuf) -> Result<(), AppError> {
  debug!("Clean: Target folder {}", path.display());
  if !path.is_dir() {
    trace!("Clean: Target folder {} does not exist", path.display());
    return Ok(());
  }

  std::fs::remove_dir_all(&path).map_err(|e| AppError::CleanIo(path, e))?;

  Ok(())
}

fn clean_path_dry(path: PathBuf) -> Result<(), AppError> {
  debug!("Clean (dry-run): Target folder {}", path.display());
  if !path.is_dir() {
    trace!(
      "Clean (dry-run): Target folder {} does not exist",
      path.display()
    );
    return Ok(());
  }

  println!(
    "Would remove {} ({})",
    path.display(),
    path.canonicalize().unwrap_or(path.clone()).display()
  );
  Ok(())
}
