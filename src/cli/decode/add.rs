use clap::Args;
use clap_complete::engine::ArgValueCompleter;
use std::path::Path;
use std::path::PathBuf;

use crate::cli::completions::artifacts::completion_artifacts_decodable;
use crate::cli::completions::codecs::completion_codecs_list_with_presets;
use crate::cli::path_glob::PathGlob;
use ludared::codecs::CodecHandlingConfidence;
use ludared::errors::app_error::AppError;
use ludared::project::Project;
use ludared::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct DecodeAddArgs {
  /// Virtual path
  #[arg(value_name = "PATH*", add = ArgValueCompleter::new(completion_artifacts_decodable))]
  virtual_path: String,

  /// Codec name
  #[arg(add = ArgValueCompleter::new(completion_codecs_list_with_presets))]
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
  let glob = PathGlob::new(&args.virtual_path);
  let mut project = Project::load_default()?;
  let matches: Vec<_> = if glob.is_glob() {
    project
      .manifest
      .artifacts()
      .into_iter()
      .filter(|path| glob.matches(path.as_str()))
      .collect()
  } else {
    vec![VirtualPath::new(
      &args.virtual_path,
    )?]
  };

  if matches.is_empty() {
    return Err(AppError::NoGlobMatches(args.virtual_path.clone()));
  }

  let (codec, preset) = match args.codec.split_once('+') {
    Some((codec, preset)) => (codec, Some(preset)),
    None => (args.codec.as_str(), None),
  };

  for vpath in matches {
    let codec = project
      .codecs
      .get(codec)
      .ok_or(AppError::CodecUnavailable(args.codec.clone()))?;
    let codec_args = match preset {
      Some(preset) => {
        let args = codec.preset_args(preset, args.args.as_deref())?;
        Some(args)
      }
      None => args.args.clone(),
    };

    let real_path = vpath.resolve(&project)?;
    let data = std::fs::read(&real_path)?;

    if vpath.is_source() {
      let source = project
        .manifest
        .sources
        .get(Path::new(vpath.as_str()))
        .ok_or_else(|| AppError::SourceNotFound(PathBuf::from(vpath.as_str())))?;

      source.sha256_matches(&data).map_err(|actual| {
        AppError::SourceFileMismatch(real_path.clone(), source.sha256.clone(), actual)
      })?;
    }

    match codec.can_handle(&data) {
      Ok(CodecHandlingConfidence::No) if !args.force => {
        return Err(AppError::CodecIncompatible(codec.id().to_owned()));
      }
      Ok(CodecHandlingConfidence::No) | Ok(CodecHandlingConfidence::Possible) => {
        eprintln!("Warning: codec handling confidence is low");
      }
      Ok(CodecHandlingConfidence::Likely) | Ok(CodecHandlingConfidence::Certain) => {}
      Err(e) => return Err(e.into()),
    }

    let decode_name = match &args.name {
      Some(name) => name.clone(),
      None => codec.decode_name(codec_args.as_deref())?,
    };
    let artifacts = codec.decode(&data, codec_args.as_deref())?;
    let codec_id = codec.id().to_owned();

    project.add_decode(
      &vpath,
      codec_id.clone(),
      &codec_args,
      decode_name,
      artifacts,
    )?;

    println!("✓ Added decode '{codec_id}' to {vpath}");
  }

  Ok(())
}
