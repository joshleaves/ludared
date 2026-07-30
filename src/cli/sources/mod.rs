use clap::{Args, Subcommand};
pub(crate) mod add;
pub(crate) mod list;
pub(crate) mod remove;
use crate::app_error::AppError;
use crate::cli::sources::add::SourcesAddArgs;
use crate::cli::sources::list::SourcesListArgs;
use crate::cli::sources::remove::SourcesRemoveArgs;

#[derive(Args)]
pub(crate) struct SourcesArgs {
  #[command(subcommand)]
  command: SourcesCommands,
}

#[derive(Subcommand)]
enum SourcesCommands {
  /// Add source file
  Add(SourcesAddArgs),

  /// List source files
  List(SourcesListArgs),

  /// Remove source file
  Remove(SourcesRemoveArgs),
}

pub(crate) fn command_sources(args: &SourcesArgs) -> Result<(), AppError> {
  match &args.command {
    SourcesCommands::List(args) => list::command_sources_list(args),
    SourcesCommands::Add(args) => add::command_sources_add(args),
    SourcesCommands::Remove(args) => remove::command_sources_remove(args),
  }
}
