use crate::cli::codecs::registry;
use crate::cli::completions::codecs::completion_codecs_list;
use clap::Args;
use clap_complete::ArgValueCompleter;
use ludared::errors::app_error::AppError;

#[derive(Args)]
pub(crate) struct CodecsInfoArgs {
  /// Codec name
  #[arg(add = ArgValueCompleter::new(completion_codecs_list))]
  codec: String,
}

pub(crate) fn command_codecs_info(args: &CodecsInfoArgs) -> Result<(), AppError> {
  let registry = registry()?;
  let Some(codec) = registry.get(&args.codec) else {
    return Err(AppError::CodecUnavailable(args.codec.clone()));
  };

  println!("Codec {}", codec.id());
  println!("- Name: \t{}", codec.name());
  println!("- Description:\t{}", codec.description());

  Ok(())
}
