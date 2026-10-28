use std::path::PathBuf;

use clap::Args;
use ludared::errors::app_error::AppError;
use ludared::project::Project;
use ludared::scripts::discovery::RESERVED_DIRECTORIES;

#[derive(Args)]
pub(crate) struct CodecsCreateArgs {
  /// New script name
  #[arg(value_hint = clap::ValueHint::FilePath)]
  codec: PathBuf,
}

pub(crate) fn command_codecs_create(args: &CodecsCreateArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;
  let scripts_root = &project.configuration.paths.scripts;

  if args.codec.is_absolute() {
    return Err(AppError::InvalidCodecScriptPath(
      "absolute paths".to_string(),
    ));
  }

  for component in args.codec.components() {
    if component == std::path::Component::ParentDir {
      return Err(AppError::InvalidCodecScriptPath("'..'".to_string()));
    }
    if RESERVED_DIRECTORIES.contains(&component.as_os_str().to_str().unwrap_or("")) {
      let path = component.as_os_str().to_string_lossy();
      return Err(AppError::InvalidCodecScriptPath(format!("'{}'", path)));
    }
  }

  let codec_path = scripts_root.join(&args.codec);
  if codec_path.exists() {
    return Err(AppError::CodecAlreadyExists(codec_path));
  }

  let contents = match codec_path.extension().and_then(|ext| ext.to_str()) {
    Some("lua") => ludared::codecs::scripts::lua::SKELETON,
    Some(ext) => return Err(AppError::UnknownCodecExtension(ext.to_string())),
    None => return Err(AppError::MissingCodecExtension),
  };

  if let Some(parent) = codec_path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  std::fs::write(codec_path, contents)?;

  Ok(())
}
