use std::path::Path;

use crate::codecs::ScriptEnvironment;
use crate::scripts::errors::ScriptError;

pub fn run_test(path: &Path, env: &ScriptEnvironment) -> Result<(), ScriptError> {
  match path.extension().and_then(|ext| ext.to_str()) {
    Some("lua") => run_test_lua(path, env),
    ext => unreachable!("Unexpected test script extension: {ext:?}"),
  }
}

fn run_test_lua(path: &Path, env: &ScriptEnvironment) -> Result<(), ScriptError> {
  let lua = mlua::Lua::new();
  let ludared = lua.create_table()?;
  for (key, value) in &env.0 {
    ludared.set(key.as_str(), value.as_str())?;
  }
  crate::codecs::scripts::lua::functions::register_functions(&lua, &ludared)?;
  lua.globals().set("LUDARED", ludared)?;

  let source = std::fs::read(path)?;

  lua.load(&source).set_name(path.to_string_lossy()).exec()?;

  Ok(())
}
