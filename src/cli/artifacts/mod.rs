use clap::Args;
use clap::Subcommand;

use crate::cli::artifacts::list::ArtifactsListArgs;
use crate::errors::app_error::AppError;
use crate::project::Project;

pub(crate) mod list;

#[derive(Args)]
pub(crate) struct ArtifactsArgs {
  #[command(subcommand)]
  command: ArtifactsCommands,
}

#[derive(Subcommand)]
enum ArtifactsCommands {
  /// List the artifacts declared in the manifest
  List(ArtifactsListArgs),
}

/// Dispatches execution to the requested `artifacts` subcommand.
///
/// Every `artifacts` subcommand is read-only, so the project is only borrowed
/// here and handed on by shared reference: they report what the manifest declares
/// and never reach the cache, the sources, or the workspace.
pub(crate) fn command_artifacts(args: &ArtifactsArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;

  match &args.command {
    ArtifactsCommands::List(args) => list::command_artifacts_list(&project, args),
  }
}
