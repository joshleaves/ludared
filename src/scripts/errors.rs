use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScriptError {
  #[error(transparent)]
  Io(#[from] std::io::Error),

  #[error(transparent)]
  LuaScriptError(#[from] mlua::Error),

  #[error("Invalid codec kind '{0}': expected 'rebuild' or 'patch'")]
  InvalidCodecKind(String),

  #[error("Invalid codec confidence '{0}': expected 'no', 'possible', 'likely' or 'certain'")]
  InvalidCodecConfidence(String),

  #[error("Missing or invalid Lua codec function '{0}'")]
  InvalidCodecFunction(String),
}
