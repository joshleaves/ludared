use clap::{Parser, Subcommand};
use log::*;
use std::process::ExitCode;
//use clap::Args;
mod app_error;
mod cli;
mod configuration;
mod formatting;
mod manifest;
mod project;
mod source;
#[cfg(test)]
mod testing;
use cli::clean::{CleanArgs, command_clean};
use cli::doctor::command_doctor;
use cli::sources::SourcesArgs;

use crate::app_error::AppError;
use crate::cli::sources::command_sources;
use crate::project::Project;

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
  /// Clean build and cache directories
  Clean(CleanArgs),

  /// Validate project configuration and sources
  Doctor,

  /// Manage source files
  Sources(SourcesArgs),
}

fn main() -> ExitCode {
  let cli = Cli::parse();
  stderrlog::new()
    .verbosity(cli.verbosity as usize)
    .init()
    .unwrap();

  match run(cli) {
    Ok(()) => ExitCode::SUCCESS,
    Err(err) => {
      error!("✗ {err}");
      // eprintln!("{err}");
      err.into()
    }
  }
}

fn run(cli: Cli) -> Result<(), AppError> {
  let project = Project::load_default()?;
  match &cli.command {
    Commands::Clean(args) => command_clean(args)?,
    Commands::Doctor => command_doctor(project)?,
    Commands::Sources(args) => command_sources(project, args)?,
  };

  Ok(())
}
