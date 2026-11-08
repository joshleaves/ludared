use clap::Args;
use log::debug;
use log::trace;
use std::path::Path;
use std::path::PathBuf;

use ludared::configuration::Configuration;
use ludared::errors::app_error::AppError;

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

  let success_message = if dry_run {
    "✓ Dry run complete"
  } else {
    "✓ All artifacts cleaned"
  };
  let clean_targets = determine_targets(&builds, &cache);
  if clean_targets.is_empty() {
    println!("{success_message}");
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
    if dry_run {
      clean_path_dry(&target);
    } else {
      clean_path(target)?;
    }
  }

  println!("{success_message}");
  Ok(())
}

fn determine_targets(builds: &Path, cache: &Path) -> Vec<(&'static str, PathBuf)> {
  let mut targets = Vec::new();

  if builds.is_dir() {
    targets.push(("builds", builds.to_path_buf()));

    let builds_abs = builds
      .canonicalize()
      .unwrap_or_else(|_| builds.to_path_buf());
    let cache_abs = cache.canonicalize().unwrap_or_else(|_| cache.to_path_buf());

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

fn clean_path_dry(path: &Path) {
  debug!("Clean (dry-run): Target folder {}", path.display());
  if !path.is_dir() {
    trace!(
      "Clean (dry-run): Target folder {} does not exist",
      path.display()
    );
    return;
  }

  println!(
    "Would remove {} ({})",
    path.display(),
    path
      .canonicalize()
      .unwrap_or_else(|_| path.to_path_buf())
      .display()
  );
}
