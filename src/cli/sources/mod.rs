use crate::app_error::AppError;
use clap::{Args, Subcommand};
pub(crate) mod list;
use crate::cli::sources::list::SourcesListArgs;

#[derive(Args)]
pub(crate) struct SourcesArgs {
  #[command(subcommand)]
  command: SourcesCommands,
}

#[derive(Subcommand)]
enum SourcesCommands {
  /// List source files
  List(SourcesListArgs),
}

pub(crate) fn command_sources(args: &SourcesArgs) -> Result<(), AppError> {
  match &args.command {
    SourcesCommands::List(args) => list::command_sources_list(args),
    // SourcesCommand::Add(args) => add::command_sources_add(args),

    // SourcesCommand::Remove(args) => remove::command_sources_remove(args),
  }
}
