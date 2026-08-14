use crate::project::Project;
use clap_complete::engine::CompletionCandidate;

pub(crate) fn complete_virtual_path(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
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
