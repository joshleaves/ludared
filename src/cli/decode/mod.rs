use crate::errors::app_error::AppError;
use clap::{Args, Subcommand};

pub(crate) mod add;

#[derive(Args)]
pub(crate) struct DecodeArgs {
  #[command(subcommand)]
  command: DecodeCommands,
}

#[derive(Subcommand)]
enum DecodeCommands {
  /// Add decode
  Add(add::DecodeAddArgs),
  // /// List decodes
  // List(DecodeListArgs),

  // /// Remove decode
  // Remove(DecodeRemoveArgs),
}

pub(crate) fn command_decode(args: &DecodeArgs) -> Result<(), AppError> {
  match &args.command {
    DecodeCommands::Add(args) => add::command_decode_add(args),
  }
}
