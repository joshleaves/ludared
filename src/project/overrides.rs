use std::fs::create_dir_all;
use std::path::{Path, PathBuf};

use log::*;

use crate::errors::app_error::AppError;
use crate::manifest::decode_node::DecodeNode;
use crate::project::cache::errors::CacheError;
use crate::virtual_path::VirtualPath;

use super::Project;

pub(crate) mod errors;

use errors::OverrideError;

/// The business layer of `ludared override`.
///
/// An override is a leaf artifact the operator owns: it is copied out of the
/// decode cache into the workspace at `WORKSPACE_ROOT/{VPATH}`, and the decode
/// node that produced it records its name so the manifest, rather than the mere
/// presence of a file, decides whether the override is active. The cache stays
/// canonical throughout and is only ever read here.
impl Project {
  /// Declares `path` as an override and materializes it in the workspace.
  ///
  /// The canonical bytes are copied out of the decode cache to
  /// `WORKSPACE_ROOT/{PATH}`, creating the parent directories the virtual path
  /// implies, and the output name is then recorded in the `overrides` field of
  /// the decode node that produced it.
  ///
  /// The copy comes first and is not undone afterwards, so the two steps are not
  /// one atomic step: a copy that fails leaves the artifact untouched and
  /// undeclared, while a manifest that cannot be written afterwards leaves the
  /// workspace file in place with nothing recording it. A later `add` refuses
  /// that file until it is passed `--force`, which is the way back out.
  ///
  /// Without `force`, an existing workspace file is left untouched and the
  /// artifact stays undeclared. `force` is the only way to overwrite one, and
  /// only for an artifact that is not already overridden: an artifact that is
  /// overridden is refreshed rather than replaced, so that it keeps its marker.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if:
  /// - `path` does not name a decoded artifact;
  /// - that artifact has decodes of its own;
  /// - that artifact is already overridden;
  /// - `path` is not indexed in the decode cache, or its blob cannot be read;
  /// - the workspace file exists and `force` was not given;
  /// - the workspace directories or the copy cannot be written;
  /// - the manifest cannot be written, by which point the workspace file has
  ///   already been written.
  pub(crate) fn add_override(&mut self, path: &VirtualPath, force: bool) -> Result<(), AppError> {
    let (node, output) = self.resolve_virtual_path(path)?;

    if !node.is_terminal_output(output) {
      return Err(OverrideError::NotTerminalArtifact(path.to_string()).into());
    }

    if node.has_override_for(output) {
      return Err(OverrideError::AlreadyOverridden(path.to_string()).into());
    }

    let blob = self.blob_of(path)?;
    let destination = self.workspace_path(path);

    if destination.exists() && !force {
      return Err(OverrideError::FileAlreadyExists(path.to_string()).into());
    }

    write_workspace_file(&blob, &destination)?;

    let (node, output) = self.manifest.resolve_virtual_path_mut(path)?;
    node.overrides.push(output);
    self.save_manifest()?;

    debug!("Overrode '{path}' into '{}'", destination.display());

    Ok(())
  }

  /// Rewrites the workspace file of an overridden artifact with its canonical
  /// bytes.
  ///
  /// This is the destructive half of the override commands: whatever the
  /// operator edited in the workspace is replaced, and a file that was deleted
  /// is recreated along with the directories it needs.
  ///
  /// The manifest and the decode cache are left untouched, so the artifact stays
  /// overridden either way.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if:
  /// - `path` does not name a decoded artifact;
  /// - that artifact is not overridden;
  /// - `path` is not indexed in the decode cache, or its blob cannot be read;
  /// - the workspace directories or the file cannot be written.
  pub(crate) fn refresh_override(&mut self, path: &VirtualPath) -> Result<(), AppError> {
    let (node, output) = self.resolve_virtual_path(path)?;

    if !node.has_override_for(output) {
      return Err(OverrideError::NotOverridden(path.to_string()).into());
    }

    let blob = self.blob_of(path)?;
    let destination = self.workspace_path(path);

    write_workspace_file(&blob, &destination)?;

    debug!("Refreshed '{path}' from its canonical bytes");

    Ok(())
  }

  /// Deactivates the override of `path`, leaving its workspace file alone.
  ///
  /// The file belongs to whoever declared the override, and is not this method's
  /// to destroy. A caller that wants it gone is expected to delete it first, so
  /// that a deletion which fails leaves the artifact overridden rather than
  /// dropping the marker of a workspace file that is still there.
  ///
  /// The manifest is the last thing written, so it either records the override as
  /// gone or the artifact stays overridden.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if:
  /// - `path` does not name a decoded artifact;
  /// - that artifact is not overridden;
  /// - the manifest cannot be written.
  pub(crate) fn remove_override(&mut self, path: &VirtualPath) -> Result<(), AppError> {
    let (node, output) = self.resolve_virtual_path(path)?;

    if !node.has_override_for(output) {
      return Err(OverrideError::NotOverridden(path.to_string()).into());
    }

    let (node, output) = self.manifest.resolve_virtual_path_mut(path)?;
    node.overrides.retain(|name| name != &output);
    self.save_manifest()?;

    debug!("Removed the override declared for '{path}'");

    Ok(())
  }

  /// Resolves the workspace file an artifact is materialized in.
  ///
  /// The workspace mirrors the virtual file system exactly: an artifact always
  /// lives at `WORKSPACE_ROOT/{VPATH}`, and there is no other destination to
  /// configure.
  pub(crate) fn workspace_path(&self, path: &VirtualPath) -> PathBuf {
    self
      .root
      .join(&self.configuration.paths.workspace)
      .join(path.to_string())
  }

  /// Resolves the decoded artifact at `path`, reporting an unresolvable path as
  /// the override business case it is rather than as a manifest failure.
  fn resolve_virtual_path(&self, path: &VirtualPath) -> Result<(&DecodeNode, &str), OverrideError> {
    self
      .manifest
      .resolve_virtual_path(path)
      .map_err(|_| OverrideError::ArtifactNotFound(path.to_string()))
  }

  /// Returns the cache blob holding the canonical bytes of `path`.
  ///
  /// The cache is read-only here, and a path the index does not name is refused
  /// rather than treated as an empty artifact.
  fn blob_of(&self, path: &VirtualPath) -> Result<PathBuf, AppError> {
    self
      .cache
      .get_entry(path)
      .ok_or_else(|| CacheError::NotIndexed(path.to_string()).into())
  }
}

/// Copies `blob` over the workspace file `destination`, creating its parents.
///
/// The copy is the only step allowed to touch operator data, and only the two
/// callers that are explicitly destructive reach it with a file that may already
/// be there.
fn write_workspace_file(blob: &Path, destination: &Path) -> Result<(), AppError> {
  if let Some(parent) = destination.parent() {
    create_dir_all(parent).map_err(|err| AppError::WorkspaceDirIo(parent.to_path_buf(), err))?;
  }

  std::fs::copy(blob, destination)
    .map_err(|err| AppError::WorkspaceFileIo(destination.to_path_buf(), err))?;

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::fixtures::lorom;
  use crate::testing::fixtures::project::ProjectFixture;

  /// The second bank, a leaf with no decode of its own.
  const SECOND_BANK: &str = "rom_bank_01.bin";

  /// A project holding the real LoROM fixture with its decode cache unpacked.
  ///
  /// The cases below work with two of the leaves it produces: the nested
  /// `TITLE.txt` and the top-level `SECOND_BANK`.
  fn unpacked() -> (ProjectFixture, String) {
    let (mut fixture, source) = lorom::project();

    fixture.project.unpack().unwrap();
    fixture.reload();

    (fixture, source)
  }

  fn vpath(path: &str) -> VirtualPath {
    VirtualPath::new(path).unwrap()
  }

  fn title(source: &str) -> VirtualPath {
    vpath(&lorom::title_vpath(source))
  }

  fn bank(source: &str) -> VirtualPath {
    vpath(&format!("{source}/{SECOND_BANK}"))
  }

  fn refuses(result: Result<(), AppError>) -> OverrideError {
    match result.unwrap_err() {
      AppError::OverrideError(err) => err,
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  fn is_artifact_not_found(err: &OverrideError, path: &VirtualPath) -> bool {
    matches!(err, OverrideError::ArtifactNotFound(reported) if reported == &path.to_string())
  }

  #[test]
  fn fails_when_the_artifact_is_not_indexed_in_the_cache() {
    // The manifest records the decode tree but nothing has been unpacked yet, so
    // there are no canonical bytes to copy out.
    let (mut fixture, source) = lorom::project();
    let path = title(&source);

    assert!(matches!(
      fixture.project.add_override(&path, false),
      Err(AppError::CacheError(CacheError::NotIndexed(_)))
    ));

    fixture.reload();
    assert!(fixture.project.manifest.overrides().is_empty());
  }

  #[test]
  fn fails_when_the_canonical_bytes_cannot_be_read() {
    let (mut fixture, source) = unpacked();
    let path = bank(&source);

    // The index still names the blob, but the blob itself is gone.
    let blob = fixture.project.cache.get_entry(&path).unwrap();
    std::fs::remove_file(&blob).unwrap();

    assert!(matches!(
      fixture.project.add_override(&path, false),
      Err(AppError::WorkspaceFileIo(_, _))
    ));
    assert!(!fixture.project.workspace_path(&path).exists());

    fixture.reload();
    assert!(!fixture.project.manifest.is_overridden(&path));
  }

  #[test]
  fn declares_nothing_when_the_workspace_file_cannot_be_written() {
    let (mut fixture, source) = unpacked();
    let path = title(&source);

    // A regular file where the workspace expects a directory.
    let workspace = fixture.project.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join(&source), b"IN THE WAY").unwrap();

    assert!(matches!(
      fixture.project.add_override(&path, false),
      Err(AppError::WorkspaceDirIo(_, _))
    ));

    fixture.reload();
    assert!(fixture.project.manifest.overrides().is_empty());
  }

  #[test]
  fn leaves_the_workspace_file_behind_when_the_manifest_cannot_be_saved() {
    let (mut fixture, source) = unpacked();
    let path = bank(&source);

    // A directory where the manifest belongs: the copy still happens, but the
    // declaration cannot be written back.
    let manifest = fixture
      .project
      .root
      .join(&fixture.project.configuration.project.manifest);
    std::fs::remove_file(&manifest).unwrap();
    std::fs::create_dir(&manifest).unwrap();

    assert!(matches!(
      fixture.project.add_override(&path, false),
      Err(AppError::ManifestFileIo(_))
    ));

    // The copy comes first and is not undone, so the file is left there with
    // nothing recording it. Nothing claims otherwise, and a later `add` refuses
    // it until it is told to force.
    assert!(fixture.project.workspace_path(&path).is_file());
  }

  #[test]
  fn refuses_to_remove_a_path_that_is_no_decoded_artifact() {
    let (mut fixture, _) = unpacked();
    let path = vpath("elsewhere.sfc/rom_bank_01.bin");

    let err = refuses(fixture.project.remove_override(&path));

    assert!(
      is_artifact_not_found(&err, &path),
      "unexpected error: {err:?}"
    );
  }

  #[test]
  fn overrides_every_leaf_of_a_tree_independently() {
    let (mut fixture, source) = unpacked();
    let nested = title(&source);
    let leaf = bank(&source);

    fixture.project.add_override(&nested, false).unwrap();
    fixture.project.add_override(&leaf, false).unwrap();

    // Declaring one says nothing about the other.
    assert_eq!(
      listed(&fixture),
      [
        lorom::title_vpath(&source),
        format!("{source}/{SECOND_BANK}")
      ]
    );

    fixture.project.remove_override(&nested).unwrap();
    assert_eq!(
      listed(&fixture),
      [format!(
        "{source}/{SECOND_BANK}"
      )]
    );
  }

  fn listed(fixture: &ProjectFixture) -> Vec<String> {
    fixture
      .project
      .manifest
      .overrides()
      .iter()
      .map(VirtualPath::to_string)
      .collect()
  }
}
