use crate::cli::cache::cat::CacheCatArgs;
use crate::cli::cache::list::CacheListArgs;
use crate::cli::cache::path::CachePathArgs;
use crate::cli::cache::tree::CacheTreeArgs;
use crate::errors::app_error::AppError;
use crate::project::Project;
use clap::{Args, Subcommand};

pub(crate) mod cat;
pub(crate) mod list;
pub(crate) mod path;
pub(crate) mod tree;

#[derive(Args)]
pub(crate) struct CacheArgs {
  #[command(subcommand)]
  command: CacheCommands,
}

#[derive(Subcommand)]
enum CacheCommands {
  /// List the virtual paths held in the decode cache
  List(CacheListArgs),

  /// Write the bytes cached for a virtual path to stdout
  Cat(CacheCatArgs),

  /// Print the path of the blob cached for a virtual path
  Path(CachePathArgs),

  /// Display the virtual paths held in the decode cache as a tree
  Tree(CacheTreeArgs),
}

/// Dispatches execution to the requested `cache` subcommand.
///
/// Every `cache` subcommand is read-only, so the project is only borrowed here
/// and handed on by shared reference: none of them may reach the index, the
/// blobs, the manifest, or the workspace.
pub(crate) fn command_cache(args: &CacheArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;
  match &args.command {
    CacheCommands::List(args) => list::command_cache_list(&project, args),
    CacheCommands::Cat(args) => cat::command_cache_cat(&project, args),
    CacheCommands::Path(args) => path::command_cache_path(&project, args),
    CacheCommands::Tree(args) => tree::command_cache_tree(&project, args),
  }
}
