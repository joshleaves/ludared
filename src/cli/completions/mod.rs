use crate::app_error::AppError;
use clap::Args;
use std::env::current_exe;
use std::path::Path;
use std::process::Command;

pub(crate) mod codecs;
pub(crate) mod sources;

#[derive(Args)]
pub(crate) struct CompletionsArgs {
  /// Shell name
  pub shell: Option<String>,
}

pub(crate) fn command_completions(args: &CompletionsArgs) -> Result<(), AppError> {
  let shell = match &args.shell {
    Some(s) => s.to_owned(),
    None => current_shell().unwrap_or_else(|| "bash".to_owned()),
  };
  let status = Command::new(current_exe()?)
    .env("COMPLETE", shell)
    .status()?;
  if !status.success() {
    // TODO: Do something later
  }

  Ok(())
}

/// Note: Shell detection is best-effort. If it fails, we silently fall back
/// to "bash", as completions are not part of Ludared's core behavior.
fn current_shell() -> Option<String> {
  match std::env::var("SHELL") {
    Err(_) => None,
    Ok(shell) => Path::new(&shell)
      .file_name()
      .and_then(|name| name.to_str())
      .map(str::to_owned),
  }
}
