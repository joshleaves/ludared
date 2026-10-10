use crate::cli::overrides::add::OverrideAddArgs;
use crate::cli::overrides::list::OverrideListArgs;
use crate::cli::overrides::refresh::OverrideRefreshArgs;
use crate::cli::overrides::remove::OverrideRemoveArgs;
use crate::cli::overrides::tree::OverrideTreeArgs;
use crate::errors::app_error::AppError;
use crate::project::Project;
use clap::{Args, Subcommand};

pub(crate) mod add;
pub(crate) mod list;
pub(crate) mod refresh;
pub(crate) mod remove;
pub(crate) mod tree;

#[derive(Args)]
pub(crate) struct OverrideArgs {
  #[command(subcommand)]
  command: OverrideCommands,
}

#[derive(Subcommand)]
enum OverrideCommands {
  /// List the artifacts declared as overrides in the manifest
  List(OverrideListArgs),

  /// Display the artifacts declared as overrides as a tree
  Tree(OverrideTreeArgs),

  /// Declare a decoded artifact as an override and materialize it in the workspace
  Add(OverrideAddArgs),

  /// Rewrite the workspace file of an overridden artifact with its canonical bytes
  Refresh(OverrideRefreshArgs),

  /// Deactivate the override of an artifact, optionally deleting its workspace file
  Remove(OverrideRemoveArgs),
}

/// Dispatches execution to the requested `override` subcommand.
///
/// `list` and `tree` only ever read the manifest, so they borrow the project
/// shared and never look at the cache or the workspace. The three commands that
/// change an override need the manifest to be written back, so they borrow it
/// mutably instead.
pub(crate) fn command_override(args: &OverrideArgs) -> Result<(), AppError> {
  let mut project = Project::load_default()?;

  match &args.command {
    OverrideCommands::List(args) => list::command_override_list(&project, args),
    OverrideCommands::Tree(args) => tree::command_override_tree(&project, args),
    OverrideCommands::Add(args) => add::command_override_add(&mut project, args),
    OverrideCommands::Refresh(args) => refresh::command_override_refresh(&mut project, args),
    OverrideCommands::Remove(args) => remove::command_override_remove(&mut project, args),
  }
}
