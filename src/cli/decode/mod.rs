use crate::errors::app_error::AppError;
use clap::{Args, Subcommand};

pub(crate) mod add;
pub(crate) mod list;
pub(crate) mod remove;

#[derive(Args)]
pub(crate) struct DecodeArgs {
  #[command(subcommand)]
  command: DecodeCommands,
}

#[derive(Subcommand)]
enum DecodeCommands {
  /// Add decode
  Add(add::DecodeAddArgs),

  /// List decodes
  List(list::DecodeListArgs),

  /// Remove decode
  Remove(remove::DecodeRemoveArgs),
}

pub(crate) fn command_decode(args: &DecodeArgs) -> Result<(), AppError> {
  match &args.command {
    DecodeCommands::Add(args) => add::command_decode_add(args),
    DecodeCommands::List(args) => list::command_decode_list(args),
    DecodeCommands::Remove(args) => remove::command_decode_remove(args),
  }
}
