use crate::app_error::AppError;
use crate::source::Source;

use log::*;
use std::path::Path;
use std::path::PathBuf;

use super::Project;

impl Project {
  /// Resolves the absolute path to a source file in the project's source directory.
  pub fn source_path(&self, source_name: &Path) -> PathBuf {
    self
      .root
      .join(&self.configuration.paths.sources)
      .join(source_name)
  }

  /// Adds a source file to the project manifest.
  ///
  /// The source file must exist in the project's source directory and must not
  /// already be present in the manifest. The manifest is updated and written to
  /// disk on success.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if:
  /// - the source file does not exist;
  /// - the source is already present in the manifest;
  /// - the source metadata cannot be read;
  /// - the manifest cannot be written.
  pub(crate) fn add_source(&mut self, source_name: &Path) -> Result<(), AppError> {
    let source_path = self.source_path(source_name);
    if !source_path.is_file() {
      warn!("File missing: {}", source_path.display());
      return Err(AppError::NoSuchFile(source_name.to_path_buf()));
    };

    if self.manifest.sources.contains_key(source_name) {
      warn!("File already in manifest: {}", &source_name.display());
      return Err(AppError::SourceAlreadyExists(source_name.to_path_buf()));
    }
    let new_source = Source::new_from_file(&source_path)?;
    self
      .manifest
      .sources
      .insert(source_name.to_path_buf(), new_source);
    self.save_manifest()?;

    Ok(())
  }

  /// Removes a source file from the project manifest.
  ///
  /// The source must already be present in the manifest. The updated manifest is
  /// written to disk on success.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if:
  /// - the source is not present in the manifest;
  /// - the manifest cannot be written.
  pub(crate) fn remove_source(&mut self, source_name: &Path) -> Result<(), AppError> {
    if self.manifest.sources.remove(source_name).is_none() {
      warn!("Source not found in manifest: {}", &source_name.display());
      return Err(AppError::SourceNotFound(source_name.to_path_buf()));
    };
    self.save_manifest()?;

    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::fixtures::project::ProjectFixture;
  use uuid::Uuid;

  #[test]
  fn add_source_adds_source_to_manifest() {
    let mut fixture = ProjectFixture::new();
    let file = format!("{}.sfc", Uuid::new_v4());
    let path = PathBuf::from(&file);
    fixture.create_source_file(&file, b"hello world");
    fixture.project.add_source(&path.clone()).unwrap();

    assert!(
      fixture
        .reload()
        .project
        .manifest
        .sources
        .contains_key(&path)
    );
  }

  #[test]
  fn add_source_fails_if_file_missing() {
    let mut fixture = ProjectFixture::new();
    let file = format!("{}.sfc", Uuid::new_v4());
    let path = PathBuf::from(&file);

    match fixture.project.add_source(&path.clone()).unwrap_err() {
      AppError::NoSuchFile(p) => assert_eq!(p, path),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn add_source_fails_if_file_already_in_manifest() {
    let mut fixture = ProjectFixture::new();
    let file = format!("{}.sfc", Uuid::new_v4());
    let path = PathBuf::from(&file);
    fixture.create_source_file(&file, b"hello world");
    fixture.project.add_source(&path.clone()).unwrap();
    fixture.reload();

    match fixture.project.add_source(&path.clone()).unwrap_err() {
      AppError::SourceAlreadyExists(p) => assert_eq!(p, path),
      err => panic!("Unexpected error: {err:?}"),
    }
  }
}
