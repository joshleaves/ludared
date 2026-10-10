use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::artifacts::complete_artifacts_decodable;
use crate::cli::completions::codecs::complete_codecs_list;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::registry::CodecRegistry;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct DecodeAddArgs {
  /// Virtual path
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_artifacts_decodable))]
  virtual_path: String,

  /// Codec name
  #[arg(add = ArgValueCompleter::new(complete_codecs_list))]
  codec: String,

  /// Codec args
  args: Option<String>,

  /// Decode node name
  #[arg(long)]
  name: Option<String>,

  /// Force usage of non-working codec
  #[arg(long)]
  force: bool,
}

pub(crate) fn command_decode_add(args: &DecodeAddArgs) -> Result<(), AppError> {
  let codec =
    CodecRegistry::get(&args.codec).ok_or(AppError::CodecUnavailable(args.codec.clone()))?;

  let vpath = VirtualPath::new(&args.virtual_path)?;
  let mut project = Project::load_default()?;
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

  let decode_name = match &args.name {
    Some(name) => name.clone(),
    None => codec.decode_name(args.args.as_deref())?,
  };
  let artifacts = codec.decode(&data, args.args.as_deref())?;

  project.add_decode(
    &vpath,
    codec.id().to_owned(),
    &args.args,
    decode_name,
    artifacts,
  )?;

  Ok(())
}
