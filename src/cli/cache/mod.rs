use crate::cli::cache::cat::CacheCatArgs;
use crate::cli::cache::path::CachePathArgs;
use crate::errors::app_error::AppError;
use crate::project::Project;
use clap::{Args, Subcommand};

pub(crate) mod cat;
pub(crate) mod path;

#[derive(Args)]
pub(crate) struct CacheArgs {
  #[command(subcommand)]
  command: CacheCommands,
}

#[derive(Subcommand)]
enum CacheCommands {
  /// Write the bytes cached for a virtual path to stdout
  Cat(CacheCatArgs),

  /// Print the path of the blob cached for a virtual path
  Path(CachePathArgs),
}

/// Dispatches execution to the requested `cache` subcommand.
///
/// Every `cache` subcommand reads the blobs the index names, so the project is
/// only borrowed here and handed on by shared reference: none of them may write
/// to the index, the blobs, the manifest, or the workspace.
pub(crate) fn command_cache(args: &CacheArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;
  match &args.command {
    CacheCommands::Cat(args) => cat::command_cache_cat(&project, args),
    CacheCommands::Path(args) => path::command_cache_path(&project, args),
  }
}
