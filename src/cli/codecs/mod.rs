use clap::Args;
use clap::Subcommand;
use std::path::Path;
use std::path::PathBuf;

use crate::cli::codecs::create::CodecsCreateArgs;
use crate::cli::codecs::detect::CodecsDetectArgs;
use crate::cli::codecs::info::CodecsInfoArgs;
use crate::cli::codecs::list::CodecsListArgs;
use crate::codecs::ScriptEnvironment;
use crate::codecs::registry::CodecRegistry;
use crate::errors::app_error::AppError;
use crate::project::Project;

pub(crate) mod create;
pub(crate) mod detect;
pub(crate) mod info;
pub(crate) mod list;

#[derive(Args)]
pub(crate) struct CodecsArgs {
  #[command(subcommand)]
  command: CodecsCommands,
}

#[derive(Subcommand)]
pub(crate) enum CodecsCommands {
  /// List all available plugins
  List(CodecsListArgs),

  /// Get Info about a codec
  Info(CodecsInfoArgs),

  // Detect codecs fit for a file
  Detect(CodecsDetectArgs),

  // Create a skeleton script file
  Create(CodecsCreateArgs),
}

/// The registry the read-only codec commands answer from.
///
/// Codecs are read from the project the way every other command reads it, so a
/// project contributes the Lua codecs its scripts directory holds alongside the
/// built-in ones. A directory holding no project is not an error for a command
/// that only reports what codecs exist: the built-in codecs are available
/// everywhere, so an absent configuration leaves them and nothing else.
///
/// An empty root names no directory at all, which is what leaves the registry
/// holding the built-in codecs on its own.
pub(crate) fn registry() -> Result<CodecRegistry, AppError> {
  if !Path::new("ludared.toml").exists() {
    return Ok(CodecRegistry::new(
      PathBuf::new(),
      ScriptEnvironment::default(),
    )?);
  }

  Ok(Project::load_default()?.codecs)
}

pub fn command_codecs(args: &CodecsArgs) -> Result<(), AppError> {
  match &args.command {
    CodecsCommands::List(args) => list::command_codecs_list(args),
    CodecsCommands::Info(args) => info::command_codecs_info(args),
    CodecsCommands::Detect(args) => detect::command_codecs_detect(args),
    CodecsCommands::Create(args) => create::command_codecs_create(args),
  }
}
