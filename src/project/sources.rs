use crate::errors::app_error::AppError;
use crate::source::Source;
use crate::virtual_path::VirtualPath;
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
  pub fn add_source(&mut self, source_name: &Path) -> Result<(), AppError> {
    let source_path = self.source_path(source_name);
    if !source_path.is_file() {
      return Err(AppError::NoSuchFile(source_name.to_path_buf()));
    };

    if self.manifest.sources.contains_key(source_name) {
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
  pub fn remove_source(&mut self, source_name: &Path) -> Result<(), AppError> {
    let source = VirtualPath::new(&source_name.to_string_lossy())?;

    if !self.manifest.sources.contains_key(source_name) {
      return Err(AppError::SourceNotFound(source_name.to_path_buf()));
    }

    // Collect cached artifacts before removing the decode tree.
    let artifacts: Vec<_> = self
      .manifest
      .artifacts()
      .into_iter()
      .filter(|path| path != &source && path.is_within(&source))
      .collect();

    let source = source_name.to_string_lossy();
    self.manifest.sources.remove(source_name);
    self.manifest.decodes.remove(source.as_ref());

    self.save_manifest()?;

    if let Err(err) = self.cache.remove_entries(&artifacts) {
      warn!(
        "Failed to clean cache for source '{}': {err}",
        source_name.display()
      );
    }

    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::fixtures::project::ProjectFixture;

  #[test]
  fn add_source_adds_source_to_manifest() {
    let mut fixture = ProjectFixture::new();
    let (file, path) = ProjectFixture::random_source_name();

    fixture.create_source_file(&file, b"hello world");
    fixture.project.add_source(&path).unwrap();

    fixture.reload();
    assert!(fixture.project.manifest.sources.contains_key(&path));
  }

  #[test]
  fn add_source_fails_if_file_missing() {
    let mut fixture = ProjectFixture::new();
    let (_, path) = ProjectFixture::random_source_name();

    match fixture.project.add_source(&path).unwrap_err() {
      AppError::NoSuchFile(p) => assert_eq!(p, path),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn add_source_fails_if_file_already_in_manifest() {
    let mut fixture = ProjectFixture::new();
    let (file, path) = ProjectFixture::random_source_name();

    fixture.create_source_file(&file, b"hello world");
    fixture.project.add_source(&path).unwrap();
    fixture.reload();

    match fixture.project.add_source(&path).unwrap_err() {
      AppError::SourceAlreadyExists(p) => assert_eq!(p, path),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn remove_source_removes_source_from_manifest() {
    let mut fixture = ProjectFixture::new();
    let (file, path) = ProjectFixture::random_source_name();

    fixture.register_source_file(&file, b"hello world");

    fixture.project.remove_source(&path).unwrap();

    fixture.reload();
    assert!(!fixture.project.manifest.sources.contains_key(&path));
  }

  #[test]
  fn remove_source_fails_if_source_not_in_manifest() {
    let mut fixture = ProjectFixture::new();
    let (_, path) = ProjectFixture::random_source_name();

    match fixture.project.remove_source(&path).unwrap_err() {
      AppError::SourceNotFound(p) => assert_eq!(p, path),
      err => panic!("Unexpected error: {err:?}"),
    }
  }
}
