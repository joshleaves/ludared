use clap_complete::engine::CompletionCandidate;

use ludared::project::Project;
use ludared::scripts::discovery::ScriptDiscovery;
use ludared::scripts::discovery::ScriptKind;

pub(crate) fn completion_scripts_test_scripts_list(
  current: &std::ffi::OsStr,
) -> Vec<CompletionCandidate> {
  let Ok(project) = Project::load_default() else {
    return Vec::new();
  };
  let scripts_root = project.configuration.paths.scripts;
  let Ok(discovery) = ScriptDiscovery::new(scripts_root) else {
    return Vec::new();
  };

  let Some(current) = current.to_str() else {
    return Vec::new();
  };

  discovery
    .retrieve_scripts(ScriptKind::Test)
    .iter()
    .filter(|test_script| test_script.starts_with(current))
    .map(|test_script| test_script.to_string_lossy().to_string())
    .map(CompletionCandidate::new)
    .collect()
}
