use clap::{Parser, Subcommand};
use log::*;
use std::process::ExitCode;
//use clap::Args;
mod app_error;
pub mod cli;
mod configuration;
mod manifest;
mod project;
use cli::clean::{CleanArgs, command_clean};
use cli::doctor::command_doctor;

use crate::app_error::AppError;

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
  match &cli.command {
    Commands::Clean(args) => command_clean(args)?,
    Commands::Doctor => command_doctor()?,
  };

  Ok(())
}
