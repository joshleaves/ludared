use clap_complete::engine::CompletionCandidate;

use crate::cli::codecs::registry;

fn completion_codecs(current: &std::ffi::OsStr, presets: bool) -> Vec<CompletionCandidate> {
  // Completion never fails a shell: a project that will not load leaves nothing
  // to offer, the same way the other project-aware completers give up.
  let Ok(registry) = registry() else {
    return Vec::new();
  };

  let Some(prefix) = current.to_str() else {
    return Vec::new();
  };

  registry
    .all_codecs()
    .flat_map(|codec| {
      let id = codec.id();
      let mut candidates = vec![id.to_owned()];

      if presets {
        candidates.extend(
          codec
            .presets()
            .iter()
            .map(|preset| format!("{id}+{preset}")),
        );
      }

      candidates
    })
    .filter(|candidate| candidate.starts_with(prefix))
    .map(CompletionCandidate::new)
    .collect()
}

pub(crate) fn completion_codecs_list(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
  completion_codecs(current, false)
}

pub(crate) fn completion_codecs_list_with_presets(
  current: &std::ffi::OsStr,
) -> Vec<CompletionCandidate> {
  completion_codecs(current, true)
}
