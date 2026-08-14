use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::codecs::complete_codecs_list;
use crate::cli::completions::virtual_path::complete_virtual_path;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::registry::CodecRegistry;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct DecodeArgs {
  /// Virtual path
  #[arg(add = ArgValueCompleter::new(complete_virtual_path))]
  virtual_path: String,

  /// Codec name
  #[arg(add = ArgValueCompleter::new(complete_codecs_list))]
  codec: String,

  /// Force usage of non-working codec
  #[arg(long)]
  force: bool,
}

pub(crate) fn command_decode(args: &DecodeArgs) -> Result<(), AppError> {
  let codec =
    CodecRegistry::get(&args.codec).ok_or(AppError::CodecUnavailable(args.codec.clone()))?;

  let vpath = VirtualPath::new(&args.virtual_path)?;
  let project = Project::load_default()?;
  let real_path = vpath.resolve(&project)?;

  let data = std::fs::read(real_path)?;
  match codec.can_handle(&data) {
    CodecHandlingConfidence::No if !args.force => {
      return Err(AppError::CodecIncompatible(codec.id().to_owned()));
    }
    CodecHandlingConfidence::No | CodecHandlingConfidence::Possible => {
      eprintln!("Warning: codec handling confidence is low");
    }
    CodecHandlingConfidence::Likely | CodecHandlingConfidence::Certain => {}
  }

  Ok(())
}
