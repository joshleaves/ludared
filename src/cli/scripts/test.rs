use log::*;
use std::collections::HashMap;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::AppError;
use crate::cli::completions::scripts::completion_scripts_test_scripts_list;
use crate::cli::path_glob::PathGlob;
use crate::codecs::ScriptEnvironment;
use crate::project::Project;
use crate::scripts::discovery::ScriptDiscovery;
use crate::scripts::discovery::ScriptKind;
use crate::scripts::errors::ScriptError;
use crate::scripts::test_runner::run_test;

#[derive(Args)]
pub(crate) struct ScriptsTestArgs {
  /// File
  #[arg(value_name = "SCRIPT*", add = ArgValueCompleter::new(completion_scripts_test_scripts_list))]
  script: Option<String>,
}

pub(crate) fn command_scripts_test(args: &ScriptsTestArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;
  let scripts_root = &project.configuration.paths.scripts;
  let discovery = ScriptDiscovery::new(scripts_root.clone())?;

  let test_scripts = discovery.retrieve_scripts(ScriptKind::Test);
  let candidates = match args.script.as_deref() {
    None => test_scripts.iter().collect(),
    Some(path) => {
      let glob = PathGlob::new(path);
      if glob.is_glob() {
        let matches: Vec<_> = test_scripts
          .iter()
          .filter(|script_path| glob.matches(&script_path.to_string_lossy()))
          .collect();
        if matches.is_empty() {
          return Err(AppError::NoGlobMatches(path.to_string()));
        }
        matches
      } else {
        match test_scripts.iter().find(|f| **f == path) {
          Some(target) => vec![target],
          None => return Err(ScriptError::ScriptNotFound(path.to_string()).into()),
        }
      }
    }
  };

  let root = project.root;
  let mut env: HashMap<String, String> = HashMap::new();
  env.insert(
    "PROJECT_ROOT".to_string(),
    root.to_string_lossy().into_owned(),
  );
  env.insert(
    "WORKSPACE".to_string(),
    root
      .join(project.configuration.paths.workspace.clone())
      .to_string_lossy()
      .into_owned(),
  );

  env.insert(
    "SCRIPTS".to_string(),
    root
      .join(project.configuration.paths.scripts.clone())
      .to_string_lossy()
      .into_owned(),
  );
  let env = ScriptEnvironment(env);

  for script_path in candidates {
    debug!("Gonna test: {}", script_path.display());
    run_test(&scripts_root.join(script_path), &env)?;
  }

  Ok(())
}
