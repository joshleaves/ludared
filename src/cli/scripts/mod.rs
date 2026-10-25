use crate::cli::scripts::test::ScriptsTestArgs;
use crate::errors::app_error::AppError;
use clap::{Args, Subcommand};

pub(crate) mod test;

#[derive(Args)]
pub(crate) struct ScriptsArgs {
  #[command(subcommand)]
  command: ScriptsCommands,
}

#[derive(Subcommand)]
enum ScriptsCommands {
  /// Run test scripts in a ludared environment
  Test(ScriptsTestArgs),
}

pub(crate) fn command_scripts(args: &ScriptsArgs) -> Result<(), AppError> {
  match &args.command {
    ScriptsCommands::Test(args) => test::command_scripts_test(args),
  }
}
