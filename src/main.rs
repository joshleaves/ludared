use clap::CommandFactory as _;
use clap::Parser;
use clap::Subcommand;
use clap_complete::CompleteEnv;
use log::*;
use std::process::ExitCode;

mod cli;
mod codecs;
mod configuration;
mod errors;
mod formatting;
mod hash;
mod manifest;
mod project;
mod scripts;
mod source;
#[cfg(test)]
mod testing;
mod virtual_path;
use cli::artifacts::ArtifactsArgs;
use cli::artifacts::command_artifacts;
use cli::build::BuildArgs;
use cli::build::command_build;
use cli::cache::CacheArgs;
use cli::cache::command_cache;
use cli::clean::CleanArgs;
use cli::clean::command_clean;
use cli::codecs::CodecsArgs;
use cli::codecs::command_codecs;
use cli::completions::CompletionsArgs;
use cli::completions::command_completions;
use cli::decode::DecodeArgs;
use cli::decode::command_decode;
use cli::doctor::command_doctor;
use cli::init::InitArgs;
use cli::init::command_init;
use cli::overrides::OverrideArgs;
use cli::overrides::command_override;
use cli::scripts::ScriptsArgs;
use cli::scripts::command_scripts;
use cli::sources::SourcesArgs;
use cli::sources::command_sources;
use cli::unpack::command_unpack;
use errors::app_error::AppError;

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
  /// Increase verbosity (-v, -vv, -vvv)
  #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
  verbosity: u8,

  #[command(subcommand)]
  command: Commands,
}

#[derive(Subcommand)]
enum Commands {
  /// Initialize a new project
  Init(InitArgs),

  /// Manage source files
  Sources(SourcesArgs),

  /// Manage codecs
  Codecs(CodecsArgs),

  /// Manage scripts
  Scripts(ScriptsArgs),

  /// Decode files
  Decode(DecodeArgs),

  /// Rebuild the decode cache from the manifest
  Unpack,

  /// Rebuild a source from its decode tree
  Build(BuildArgs),

  /// Inspect the artifacts declared in the manifest
  Artifacts(ArtifactsArgs),

  /// Inspect the decode cache
  Cache(CacheArgs),

  /// Manage the decoded artifacts materialized in the workspace
  Override(OverrideArgs),

  /// Validate project configuration and sources
  Doctor,

  /// Clean build and cache directories
  Clean(CleanArgs),

  /// Create completions for your shell
  Completions(CompletionsArgs),
}

fn main() -> ExitCode {
  CompleteEnv::with_factory(Cli::command).complete();

  let cli = Cli::parse();
  stderrlog::new()
    .verbosity(cli.verbosity as usize)
    .init()
    .unwrap();

  #[cfg(feature = "profiling")]
  let _profiling_guard = {
    use tracing_subscriber::prelude::*;

    let (layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
      .include_args(true)
      .build();
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer)).unwrap();
    guard
  };

  match run(cli) {
    Ok(()) => ExitCode::SUCCESS,
    Err(err) => {
      error!("✗ {err}");
      ExitCode::FAILURE
    }
  }
}

fn run(cli: Cli) -> Result<(), AppError> {
  match cli.command {
    Commands::Completions(args) => command_completions(&args)?,
    Commands::Artifacts(args) => command_artifacts(&args)?,
    Commands::Cache(args) => command_cache(&args)?,
    Commands::Clean(args) => command_clean(&args)?,
    Commands::Codecs(args) => command_codecs(&args)?,
    Commands::Decode(args) => command_decode(&args)?,
    Commands::Doctor => command_doctor()?,
    Commands::Init(mut args) => command_init(&mut args)?,
    Commands::Override(args) => command_override(&args)?,
    Commands::Scripts(args) => command_scripts(&args)?,
    Commands::Sources(args) => command_sources(&args)?,
    Commands::Unpack => command_unpack()?,
    Commands::Build(args) => command_build(&args)?,
  }

  Ok(())
}
