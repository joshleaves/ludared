use crate::project::Project;
use clap_complete::engine::CompletionCandidate;
use std::path::PathBuf;

pub(crate) fn complete_source_add(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
  let Ok(project) = Project::load_default() else {
    return Vec::new();
  };
  let sources_path = &project.configuration.paths.sources;
  let Ok(entries) = std::fs::read_dir(sources_path) else {
    return Vec::new();
  };
  entries
    .filter_map(Result::ok)
    .filter(|entry| entry.path().is_file())
    .map(|entry| PathBuf::from(entry.file_name()))
    .filter(|source_name| !project.manifest.sources.contains_key(source_name))
    .filter(|source_name| source_name.starts_with(current))
    .map(CompletionCandidate::new)
    .collect()
}

pub(crate) fn complete_source_remove(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
  let Ok(project) = Project::load_default() else {
    return Vec::new();
  };
  project
    .manifest
    .sources
    .keys()
    .filter(|source_name| source_name.starts_with(current))
    .cloned()
    .map(CompletionCandidate::new)
    .collect()
}
