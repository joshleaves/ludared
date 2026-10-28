use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::artifacts::completion_artifacts_list;
use crate::cli::path_glob::PathGlob;
use ludared::codecs::CodecHandlingConfidence;
use ludared::errors::app_error::AppError;
use ludared::project::Project;
use ludared::virtual_path::VirtualPath;

#[derive(Clone, Copy, clap::ValueEnum)]
enum DetectionLevel {
  /// "I may extract something out of that data, maybe garbage, maybe gold."
  Possible,
  /// "All signs point to yes, but false positives are possible."
  Likely,
  /// "I know that data, I know that format, I'm in charge."
  Certain,
}

#[allow(clippy::from_over_into)]
impl Into<CodecHandlingConfidence> for DetectionLevel {
  fn into(self) -> CodecHandlingConfidence {
    match self {
      Self::Possible => CodecHandlingConfidence::Possible,
      Self::Likely => CodecHandlingConfidence::Likely,
      Self::Certain => CodecHandlingConfidence::Certain,
    }
  }
}

#[derive(Args)]
pub(crate) struct CodecsDetectArgs {
  /// File
  #[arg(value_name = "PATH*", add = ArgValueCompleter::new(completion_artifacts_list))]
  virtual_path: Option<String>,

  /// Minimum codec detection confidence
  #[arg(long)]
  level: Option<DetectionLevel>,
}

pub(crate) fn command_codecs_detect(args: &CodecsDetectArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;
  let virtual_path = args.virtual_path.as_deref().unwrap_or("**");

  let glob = PathGlob::new(virtual_path);
  let matches: Vec<_> = if glob.is_glob() {
    project
      .manifest
      .artifacts()
      .into_iter()
      .filter(|path| project.manifest.is_terminal(path))
      .filter(|path| glob.matches(path.as_str()))
      .collect()
  } else {
    vec![VirtualPath::new(
      virtual_path,
    )?]
  };

  if matches.is_empty() {
    return Err(AppError::NoGlobMatches(virtual_path.to_string()));
  }

  let level: CodecHandlingConfidence = match (args.level, glob.is_glob()) {
    (Some(level), _) => level.into(),
    (None, true) => CodecHandlingConfidence::Certain,
    (None, false) => CodecHandlingConfidence::Possible,
  };

  for vpath in matches {
    let artifact_path = vpath.resolve(&project)?;
    let data = std::fs::read(artifact_path)?;
    for (codec, confidence) in project.codecs.detect(&data)? {
      if confidence < level {
        continue;
      }
      println!("{}\t{:>8}: {}", vpath, confidence, codec.id());
    }
  }

  Ok(())
}
