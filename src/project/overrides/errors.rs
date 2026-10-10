use thiserror::Error;

/// The business cases an override command has to tell apart.
///
/// Each variant names the situation rather than the command that hit it, and
/// carries the advice the operator needs to get unstuck.
#[derive(Error, Debug)]
pub(crate) enum OverrideError {
  #[error("Artifact not found: '{0}'")]
  ArtifactNotFound(String),

  #[error("Cannot override '{0}': artifact is not terminal")]
  NotTerminalArtifact(String),

  #[error("Artifact '{0}' is already overridden (use 'override refresh {0}' to reset it)")]
  AlreadyOverridden(String),

  #[error("Cannot decode '{0}': artifact is overridden (use 'override remove {0}' first)")]
  OverriddenArtifactCannotBeDecoded(String),

  #[error("Artifact '{0}' is not overridden")]
  NotOverridden(String),

  #[error("Workspace file already exists for '{0}' (use --force to overwrite)")]
  FileAlreadyExists(String),
}
