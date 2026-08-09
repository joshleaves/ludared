use crate::cli::completions::codecs::complete_codecs_info;
use crate::codecs::registry::CodecRegistry;
use crate::errors::app_error::AppError;
use clap::Args;
use clap_complete::ArgValueCompleter;

#[derive(Args)]
pub(crate) struct CodecsInfoArgs {
  /// Codec name
  #[arg(add = ArgValueCompleter::new(complete_codecs_info))]
  codec: String,
}

pub(crate) fn command_codecs_info(args: &CodecsInfoArgs) -> Result<(), AppError> {
  let Some(codec) = CodecRegistry::builtins()
    .iter()
    .find(|codec| codec.id() == args.codec)
  else {
    return Err(AppError::CodecUnavailable(args.codec.clone()));
  };
  println!("Codec {}", codec.id());
  println!("- Name: \t{}", codec.name());
  println!("- Description:\t{}", codec.description());

  Ok(())
}
