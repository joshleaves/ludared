use clap_complete::engine::CompletionCandidate;
use ludared::project::Project;
use ludared::virtual_path::VirtualPath;
use std::ffi::OsStr;

pub(crate) fn completion_override_list(current: &OsStr) -> Vec<CompletionCandidate> {
  let Ok(project) = Project::load_default() else {
    return Vec::new();
  };

  let Some(prefix) = current.to_str() else {
    return Vec::new();
  };

  // A listing is filtered by a branch, so the parents leading to an override are
  // offered as well, even though no decode node ever produced them.
  let mut roots: Vec<VirtualPath> = project
    .manifest
    .overrides()
    .iter()
    .flat_map(VirtualPath::ancestors)
    .collect();

  roots.sort_by_key(|root| root.to_string());
  roots.dedup();

  roots
    .iter()
    .filter(|root| root.has_text_prefix(prefix))
    .map(|root| root.to_string())
    .map(CompletionCandidate::new)
    .collect()
}

pub(crate) fn completion_override_addable(current: &OsStr) -> Vec<CompletionCandidate> {
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
    // What an override may be declared on is the manifest's call, not this
    // command's: the same rules `override add` goes on to enforce.
    .filter(|artifact| {
      project.manifest.is_terminal(artifact) && !project.manifest.is_overridden(artifact)
    })
    .filter(|artifact| artifact.has_text_prefix(prefix))
    .map(|artifact| artifact.to_string())
    .map(CompletionCandidate::new)
    .collect()
}

pub(crate) fn completion_override_active(current: &OsStr) -> Vec<CompletionCandidate> {
  let Ok(project) = Project::load_default() else {
    return Vec::new();
  };

  let Some(prefix) = current.to_str() else {
    return Vec::new();
  };

  project
    .manifest
    .overrides()
    .iter()
    .filter(|vpath| vpath.has_text_prefix(prefix))
    .map(|vpath| vpath.to_string())
    .map(CompletionCandidate::new)
    .collect()
}
