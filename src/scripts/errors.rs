use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScriptError {
  #[error(transparent)]
  Io(#[from] std::io::Error),

  #[error(transparent)]
  Lua(#[from] mlua::Error),

  #[error("Script not found: {0}")]
  ScriptNotFound(String),

  #[error("Invalid codec kind '{0}': expected 'rebuild' or 'patch'")]
  InvalidCodecKind(String),

  #[error("Invalid codec confidence '{0}': expected 'no', 'possible', 'likely' or 'certain'")]
  InvalidCodecConfidence(String),

  #[error("Missing or invalid Lua codec function '{0}'")]
  InvalidCodecFunction(String),
}
