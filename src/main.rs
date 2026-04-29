use clap::{Parser, Subcommand};
use std::process::ExitCode;
//use clap::Args;
mod app_error;
pub mod cli;
mod configuration;
use cli::doctor::command_doctor;

use crate::app_error::AppError;

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
  #[command(subcommand)]
  command: Commands,
}

#[derive(Subcommand)]
enum Commands {
  /// Adds files to myapp
  // Add(AddArgs),
  Doctor,
}

// #[derive(Args)]
// struct AddArgs {
//   name: Option<String>,
// }

fn main() -> ExitCode {
  match run() {
    Ok(()) => ExitCode::SUCCESS,
    Err(err) => {
      eprintln!("{err}");
      err.into()
    }
  }
}

fn run() -> Result<(), AppError> {
  let cli = Cli::parse();

  match &cli.command {
    Commands::Doctor => command_doctor()?,
  };

  Ok(())
}
