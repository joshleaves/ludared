use crate::cli::codecs::detect::CodecsDetectArgs;
use crate::cli::codecs::info::CodecsInfoArgs;
use crate::errors::app_error::AppError;
use clap::{Args, Subcommand};

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
  List,

  /// Get Info about a codec
  Info(CodecsInfoArgs),

  // // Detect codecs fit for a file
  Detect(CodecsDetectArgs),
}
pub fn command_codecs(args: &CodecsArgs) -> Result<(), AppError> {
  match &args.command {
    CodecsCommands::List => list::command_codecs_list(),
    CodecsCommands::Info(args) => info::command_codecs_info(args),
    CodecsCommands::Detect(args) => detect::command_codecs_detect(args),
  }
}
