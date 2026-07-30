use crate::{app_error::AppError, project::Project, source::Source};
use clap::Args;
use log::*;
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct SourcesAddArgs {
  /// Source file to add
  file: PathBuf,
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
pub(crate) fn command_sources_add(
  project: &mut Project,
  args: &SourcesAddArgs,
) -> Result<(), AppError> {
  let source_path = project.source_path(&args.file);
  if !source_path.is_file() {
    warn!("File missing: {}", source_path.display());
    return Err(AppError::NoSuchFile(args.file.clone()));
  };

  if project.manifest.sources.contains_key(&args.file) {
    warn!("File already in manifest: {}", &args.file.display());
    return Err(AppError::SourceAlreadyExists(args.file.clone()));
  }
  let new_source = Source::new_from_file(&source_path)?;
  project
    .manifest
    .sources
    .insert(args.file.clone(), new_source);
  project.save_manifest()?;

  println!("✓ Added source {}", &args.file.display());

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::fixtures::project::ProjectFixture;
  use uuid::Uuid;

  #[test]
  fn adds_source_to_manifest() {
    let mut fixture = ProjectFixture::new();
    let file = format!("{}.sfc", Uuid::new_v4());
    let path = PathBuf::from(&file);
    fixture.create_source_file(&file, b"hello world");
    let args = SourcesAddArgs { file: path.clone() };
    command_sources_add(&mut fixture.project, &args).unwrap();

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
  fn fails_if_file_missing() {
    let mut fixture = ProjectFixture::new();
    let file = format!("{}.sfc", Uuid::new_v4());
    let path = PathBuf::from(&file);
    let args = SourcesAddArgs { file: path.clone() };

    match command_sources_add(&mut fixture.project, &args).unwrap_err() {
      AppError::NoSuchFile(p) => assert_eq!(p, path),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn fails_if_file_already_in_manifest() {
    let mut fixture = ProjectFixture::new();
    let file = format!("{}.sfc", Uuid::new_v4());
    let path = PathBuf::from(&file);
    fixture.create_source_file(&file, b"hello world");
    let args = SourcesAddArgs { file: path.clone() };
    command_sources_add(&mut fixture.project, &args).unwrap();
    fixture.reload();

    match command_sources_add(&mut fixture.project, &args).unwrap_err() {
      AppError::SourceAlreadyExists(p) => assert_eq!(p, path),
      err => panic!("Unexpected error: {err:?}"),
    }
  }
}
