use crate::project::Project;
use clap_complete::engine::CompletionCandidate;
use std::ffi::OsStr;

pub(crate) fn completion_artifacts_list(current: &OsStr) -> Vec<CompletionCandidate> {
  let Ok(project) = Project::load_default() else {
    return Vec::new();
  };

  let Some(prefix) = current.to_str() else {
    return Vec::new();
  };

  project
    .manifest
    .artifacts()
    .iter()
    .filter(|vpath| vpath.has_text_prefix(prefix))
    .map(|vpath| vpath.to_string())
    .map(CompletionCandidate::new)
    .collect()
}

pub(crate) fn completion_artifacts_decodable(
  current: &std::ffi::OsStr,
) -> Vec<CompletionCandidate> {
  let Ok(project) = Project::load_default() else {
    return Vec::new();
  };

  let Some(current) = current.to_str() else {
    return Vec::new();
  };

  let mut results: Vec<CompletionCandidate> = vec![];
  results.extend(
    project
      .manifest
      .sources
      .keys()
      .filter(|source_name| source_name.to_string_lossy().starts_with(current))
      .cloned()
      .map(CompletionCandidate::new),
  );

  results.extend(
    project
      .cache
      .entries()
      .filter(|source_name| source_name.starts_with(current))
      .map(CompletionCandidate::new),
  );

  results
}

pub(crate) fn completion_artifacts_decoded(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
  let Ok(project) = Project::load_default() else {
    return Vec::new();
  };

  let Some(current) = current.to_str() else {
    return Vec::new();
  };

  project
    .manifest
    .decodes()
    .into_iter()
    .filter(|vpath| vpath.has_text_prefix(current))
    .map(|vpath| vpath.to_string())
    .map(CompletionCandidate::new)
    .collect()
}
