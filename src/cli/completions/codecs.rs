use crate::cli::codecs::registry;
use clap_complete::engine::CompletionCandidate;

pub(crate) fn completion_codecs_list(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
  // Completion never fails a shell: a project that will not load leaves nothing
  // to offer, the same way the other project-aware completers give up.
  let Ok(registry) = registry() else {
    return Vec::new();
  };

  registry
    .all_codecs()
    .filter(|codec| {
      current
        .to_str()
        .is_some_and(|prefix| codec.id().starts_with(prefix))
    })
    .map(|codec| codec.id().to_owned())
    .map(CompletionCandidate::new)
    .collect()
}
