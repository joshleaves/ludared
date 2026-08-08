use clap::CommandFactory as _;
use clap::{Parser, Subcommand};
use clap_complete::CompleteEnv;
use log::*;
use std::process::ExitCode;

mod app_error;
mod cli;
mod configuration;
mod formatting;
mod manifest;
mod project;
mod source;
#[cfg(test)]
mod testing;
use crate::app_error::AppError;
use cli::clean::{CleanArgs, command_clean};
use cli::doctor::command_doctor;
use cli::init::{InitArgs, command_init};
use cli::sources::{SourcesArgs, command_sources};

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

  /// Validate project configuration and sources
  Doctor,

  /// Clean build and cache directories
  Clean(CleanArgs),
}

fn main() -> ExitCode {
  CompleteEnv::with_factory(Cli::command).complete();

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
  match cli.command {
    Commands::Clean(args) => command_clean(&args)?,
    Commands::Doctor => command_doctor()?,
    Commands::Init(mut args) => command_init(&mut args)?,
    Commands::Sources(args) => command_sources(&args)?,
  }

  Ok(())
}
