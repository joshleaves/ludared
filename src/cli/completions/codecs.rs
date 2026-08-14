use crate::codecs::registry::CodecRegistry;
use clap_complete::engine::CompletionCandidate;

pub(crate) fn complete_codecs_list(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
  CodecRegistry::builtins()
    .iter()
    .filter(|codec| {
      current
        .to_str()
        .is_some_and(|prefix| codec.id().starts_with(prefix))
    })
    .map(|codec| codec.id())
    .map(CompletionCandidate::new)
    .collect()
}
