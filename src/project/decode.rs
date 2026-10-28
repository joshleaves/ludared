use log::*;

use crate::codecs::DecodedArtifact;
use crate::errors::app_error::AppError;
use crate::manifest::decode_node::DecodeNode;
use crate::virtual_path::VirtualPath;

use super::Project;

impl Project {
  /// Adds a decode operation to the manifest and stores the artifacts it produced.
  ///
  /// The manifest half is [`Manifest::add_decode`]: it checks the decode and the
  /// names it wants to record, and rejects the operation before anything is
  /// written. What is accepted is then written into this project's decode cache,
  /// and only once that has succeeded is the manifest written down, so a cache
  /// that cannot be written to leaves no declaration behind to disagree with it.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if the manifest refuses the decode, if an artifact
  /// cannot be written to the decode cache or the cache index cannot be saved, or
  /// if the manifest cannot be written. On any of those the manifest is left as
  /// it was on disk.
  pub fn add_decode(
    &mut self,
    path: &VirtualPath,
    codec_id: String,
    args: &Option<String>,
    name: String,
    artifacts: Vec<DecodedArtifact>,
  ) -> Result<(), AppError> {
    debug!("Adding decode '{}' on '{path}'", name);

    self
      .manifest
      .add_decode(&self.codecs, path, codec_id, args, name, &artifacts)?;

    for artifact in &artifacts {
      let artifact_path = path.join(&artifact.name)?;
      self.cache.add_entry(&artifact_path, &artifact.data)?;
    }
    self.cache.save()?;

    self.save_manifest()
  }

  /// Removes a decode from the manifest and forgets the artifacts it produced.
  ///
  /// The logical half is [`Manifest::remove_decode`]: it selects the decode,
  /// refuses a non-terminal one, and drops it along with any overrides it
  /// declared. All path calculations happen before the manifest is persisted,
  /// and the manifest is persisted before anything else is touched.
  ///
  /// Once the manifest is saved the removal is logically complete. The outputs
  /// are removed from the cache index and that index is saved before any blob is
  /// deleted. A blob is deleted only when no remaining index entry names it, and
  /// that physical cleanup is best-effort: an orphaned blob is acceptable, a
  /// persisted index pointing at a missing blob is not. If the cache index cannot
  /// be saved, its entries are rolled back and the removal still succeeds; the
  /// stale index is only an orphan. With `clean`, the workspace files of the
  /// lifted overrides are deleted the same way, after the manifest stopped
  /// declaring them.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if the manifest refuses the removal, an output path
  /// is invalid, or the manifest cannot be written. Cache index, workspace, and
  /// blob cleanup errors are logged but do not fail a removal whose manifest has
  /// already been persisted.
  pub fn remove_decode(
    &mut self,
    path: &VirtualPath,
    name: Option<&str>,
    clean: bool,
  ) -> Result<DecodeNode, AppError> {
    debug!("Removing a decode under '{path}'");

    let removed = self.manifest.remove_decode(path, name)?;

    let outputs = removed
      .outputs
      .iter()
      .map(|output| path.join(output))
      .collect::<Result<Vec<_>, _>>()?;

    let workspace_files = removed
      .overrides
      .iter()
      .map(|output| Ok(self.workspace_path(&path.join(output)?)))
      .collect::<Result<Vec<_>, AppError>>()?;

    // Persist the manifest before invalidating cache entries.
    self.save_manifest()?;

    // The manifest is durable now, so the removal is logically complete. Saving
    // the cache index before deleting blobs can leave an orphan but never a
    // dangling index entry. If that save fails, the cache rolls back and the
    // stale entry is an acceptable orphan.
    if let Err(err) = self.cache.remove_entries(&outputs) {
      warn!("Could not update the decode cache index after removing '{path}': {err}");
    }

    // Cache cleanup is best-effort after manifest persistence.
    if clean {
      for destination in &workspace_files {
        match std::fs::remove_file(destination) {
          Ok(()) => debug!("Deleted workspace file '{}'", destination.display()),
          Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
          Err(err) => warn!(
            "Could not clean workspace file '{}': {err}",
            destination.display()
          ),
        }
      }
    }

    Ok(removed)
  }
}

#[cfg(test)]
mod tests {
  use crate::testing::fixtures::project::ProjectFixture;
  use crate::virtual_path::VirtualPath;

  use super::*;

  /// A project whose manifest records `source` and nothing else.
  fn project() -> (ProjectFixture, String) {
    let (source, path) = ProjectFixture::random_source_name();

    let mut fixture = ProjectFixture::new();
    fixture.create_source_file(&path, b"HELLOWORLD");
    fixture.write_manifest(format!(
      r#"{{ "sources": {{ "{source}": {{ "sha256": "0b21b7db59cd154904fac6336fa7d2be1bab38d632794f281549584068cdcb74", "size": null, "label": null }} }} }}"#,
    ));
    fixture.reload();

    (fixture, source)
  }

  fn artifacts(names: &[&str]) -> Vec<DecodedArtifact> {
    names
      .iter()
      .map(|name| DecodedArtifact {
        name: (*name).to_owned(),
        data: b"HELLO".to_vec(),
      })
      .collect()
  }

  fn vpath(path: &str) -> VirtualPath {
    VirtualPath::new(path).unwrap()
  }

  /// The manifest as it stands on disk.
  fn written(fixture: &ProjectFixture) -> String {
    std::fs::read_to_string(
      fixture
        .project
        .root
        .join(&fixture.project.configuration.project.manifest),
    )
    .unwrap()
  }

  #[test]
  fn registers_a_decode_and_stores_what_it_produced() {
    let (mut fixture, source) = project();

    fixture
      .project
      .add_decode(
        &vpath(&source),
        "std/generic/extract".to_owned(),
        &None,
        "head".to_owned(),
        artifacts(&[
          "head.bin",
          "TITLE.txt",
        ]),
      )
      .unwrap();

    // The artifacts are cached under the virtual paths the decode recorded.
    for name in [
      "head.bin",
      "TITLE.txt",
    ] {
      let path = vpath(&format!("{source}/{name}"));

      assert_eq!(
        std::fs::read(fixture.project.cache.get_entry(&path).unwrap()).unwrap(),
        b"HELLO",
        "'{name}' should be cached"
      );
    }
  }

  #[test]
  fn writes_the_decode_down_to_the_manifest() {
    let (mut fixture, source) = project();

    fixture
      .project
      .add_decode(
        &vpath(&source),
        "std/generic/extract".to_owned(),
        &Some(r#"{"offset":0}"#.to_owned()),
        "head".to_owned(),
        artifacts(&["head.bin"]),
      )
      .unwrap();

    // What was persisted is what a fresh read of the project sees.
    fixture.reload();

    let (node, output) = fixture
      .project
      .manifest
      .resolve_virtual_path(&vpath(&format!("{source}/head.bin")))
      .unwrap();

    assert_eq!(node.name, "head");
    assert_eq!(output, "head.bin");
    // The arguments are recorded verbatim, so what the codec was handed is what a
    // later read of the manifest hands back.
    assert_eq!(node.codec.args.as_deref(), Some(r#"{"offset":0}"#));
  }

  #[test]
  fn records_arbitrary_arguments_unchanged() {
    // Nothing about an argument is ludared's to interpret, so each of these has
    // to reach the manifest and come back out of it byte for byte. None is JSON
    // the manifest could have parsed, and the spacing and key order of the ones
    // that happen to be are part of the string just as much as the content is.
    for args in [
      // Not JSON at all.
      "offset = 2, length = 4",
      "head.bin\t42",
      // JSON whose spacing and key order would not survive being parsed.
      "{ \"zebra\": 1,\n   \"apple\":   [2,  3],\n  \"nested\": {\"b\":1,\"a\":2} }",
      // A bare scalar, and a string that merely looks like JSON.
      "42",
      "{\"offset\":0}",
    ] {
      let (mut fixture, source) = project();

      fixture
        .project
        .add_decode(
          &vpath(&source),
          "std/generic/extract".to_owned(),
          &Some(args.to_owned()),
          "head".to_owned(),
          artifacts(&["head.bin"]),
        )
        .unwrap();

      fixture.reload();

      let (node, _) = fixture
        .project
        .manifest
        .resolve_virtual_path(&vpath(&format!("{source}/head.bin")))
        .unwrap();

      assert_eq!(node.codec_args(), Some(args), "recorded as {args:?}");
    }
  }

  #[test]
  fn records_an_absent_argument_apart_from_an_empty_one() {
    let recorded = |args: Option<&str>| {
      let (mut fixture, source) = project();

      fixture
        .project
        .add_decode(
          &vpath(&source),
          "std/generic/extract".to_owned(),
          &args.map(str::to_owned),
          "head".to_owned(),
          artifacts(&["head.bin"]),
        )
        .unwrap();

      fixture.reload();

      let (node, _) = fixture
        .project
        .manifest
        .resolve_virtual_path(&vpath(&format!("{source}/head.bin")))
        .unwrap();

      node.codec_args().map(str::to_owned)
    };

    // `decode add` given no argument at all, and given an empty one, are two
    // different operations and stay two different things in the manifest.
    assert_eq!(recorded(None), None);
    assert_eq!(recorded(Some("")), Some(String::new()));
  }

  #[test]
  fn leaves_the_manifest_alone_when_the_cache_cannot_be_written() {
    let (mut fixture, source) = project();
    let before = written(&fixture);

    // A cache root that was never created cannot be written into.
    fixture.project.cache.root = fixture.temp.path().join("never-created");

    assert!(
      fixture
        .project
        .add_decode(
          &vpath(&source),
          "std/generic/extract".to_owned(),
          &None,
          "head".to_owned(),
          artifacts(&["head.bin"]),
        )
        .is_err()
    );

    // Nothing was declared that the cache cannot back up.
    assert_eq!(written(&fixture), before);
    assert!(!before.contains("head.bin"));
  }

  #[test]
  fn refuses_a_decode_the_manifest_will_not_accept() {
    let (mut fixture, source) = project();

    assert!(
      fixture
        .project
        .add_decode(
          &vpath(&source),
          "std/generic/extract".to_owned(),
          &None,
          "head".to_owned(),
          artifacts(&[
            "data",
            "data/gfx/head.bin"
          ]),
        )
        .is_err()
    );

    // A refused decode leaves neither the cache nor the manifest touched.
    fixture.reload();

    assert!(fixture.project.cache.entries().next().is_none());
    assert!(!written(&fixture).contains("gfx"));
  }

  #[test]
  fn refuses_a_decode_on_a_path_that_is_no_artifact() {
    let (mut fixture, _) = project();
    let before = written(&fixture);

    assert!(
      fixture
        .project
        .add_decode(
          &vpath("elsewhere.sfc/head.bin"),
          "std/generic/extract".to_owned(),
          &None,
          "head".to_owned(),
          artifacts(&["head.bin"]),
        )
        .is_err()
    );

    assert_eq!(written(&fixture), before);
  }

  #[test]
  fn removes_a_decode_from_the_manifest_and_cache_index() {
    let (mut fixture, source) = project();
    let head = vpath(&format!("{source}/head.bin"));

    fixture
      .project
      .add_decode(
        &vpath(&source),
        "std/generic/extract".to_owned(),
        &None,
        "head".to_owned(),
        artifacts(&["head.bin"]),
      )
      .unwrap();

    let blob = fixture.project.cache.get_entry(&head).unwrap();

    let removed = fixture
      .project
      .remove_decode(&vpath(&source), None, false)
      .unwrap();

    assert_eq!(removed.name, "head");
    assert_eq!(removed.outputs, ["head.bin"]);

    fixture.reload();

    // The artifact was the blob's only reference, so both the index entry and
    // the content-addressed blob are gone.
    assert!(fixture.project.cache.get_entry(&head).is_none());
    assert!(!blob.exists());
    assert!(!written(&fixture).contains("head.bin"));
  }

  #[test]
  fn leaves_the_cache_and_workspace_alone_when_the_manifest_cannot_be_saved() {
    let (mut fixture, source) = project();
    let head = vpath(&format!("{source}/head.bin"));

    fixture
      .project
      .add_decode(
        &vpath(&source),
        "std/generic/extract".to_owned(),
        &None,
        "head".to_owned(),
        artifacts(&["head.bin"]),
      )
      .unwrap();
    fixture.project.add_override(&head, false).unwrap();

    let blob = fixture.project.cache.get_entry(&head).unwrap();
    let workspace = fixture.project.workspace_path(&head);
    let cache: Vec<String> = fixture.project.cache.entries().map(str::to_owned).collect();
    let index = fixture.project.cache.root.join("index.json");
    let index_before = std::fs::read(&index).unwrap();

    // A directory where the manifest belongs cannot be written as a file.
    let manifest_path = fixture
      .project
      .root
      .join(&fixture.project.configuration.project.manifest);
    std::fs::remove_file(&manifest_path).unwrap();
    std::fs::create_dir(&manifest_path).unwrap();

    assert!(
      fixture
        .project
        .remove_decode(&vpath(&source), None, true)
        .is_err()
    );

    // The failed manifest write happens before anything physical is touched.
    assert_eq!(
      fixture
        .project
        .cache
        .entries()
        .map(str::to_owned)
        .collect::<Vec<_>>(),
      cache
    );
    assert_eq!(std::fs::read(&index).unwrap(), index_before);
    assert!(blob.is_file());
    assert!(workspace.is_file());
  }

  #[test]
  fn removes_the_decode_when_the_cache_index_cannot_be_saved() {
    let (mut fixture, source) = project();
    let head = vpath(&format!("{source}/head.bin"));

    fixture
      .project
      .add_decode(
        &vpath(&source),
        "std/generic/extract".to_owned(),
        &None,
        "head".to_owned(),
        artifacts(&["head.bin"]),
      )
      .unwrap();
    fixture.project.add_override(&head, false).unwrap();

    let blob = fixture.project.cache.get_entry(&head).unwrap();
    let workspace = fixture.project.workspace_path(&head);
    let cache: Vec<String> = fixture.project.cache.entries().map(str::to_owned).collect();

    // A directory where the cache index belongs cannot be written as a file.
    let index = fixture.project.cache.root.join("index.json");
    std::fs::remove_file(&index).unwrap();
    std::fs::create_dir(&index).unwrap();

    let removed = fixture
      .project
      .remove_decode(&vpath(&source), None, true)
      .unwrap();

    assert_eq!(removed.name, "head");

    // The manifest is persisted without the decode, even though the cache index
    // could not be. The cache rolled back, leaving its entry and blob as an
    // acceptable orphan.
    assert!(!written(&fixture).contains("head.bin"));
    assert_eq!(
      fixture
        .project
        .cache
        .entries()
        .map(str::to_owned)
        .collect::<Vec<_>>(),
      cache
    );
    assert_eq!(fixture.project.cache.get_entry(&head), Some(blob.clone()));
    assert!(blob.is_file());

    // Physical cleanup still ran after the cache-index failure.
    assert!(!workspace.exists());
  }

  #[test]
  fn removes_a_decode_even_when_its_blob_cannot_be_deleted() {
    let (mut fixture, source) = project();
    let head = vpath(&format!("{source}/head.bin"));

    fixture
      .project
      .add_decode(
        &vpath(&source),
        "std/generic/extract".to_owned(),
        &None,
        "head".to_owned(),
        artifacts(&["head.bin"]),
      )
      .unwrap();

    let blob = fixture.project.cache.get_entry(&head).unwrap();

    // A directory where the blob belongs cannot be removed as a file.
    std::fs::remove_file(&blob).unwrap();
    std::fs::create_dir(&blob).unwrap();

    fixture
      .project
      .remove_decode(&vpath(&source), None, false)
      .unwrap();

    // The index was persisted without the entry before physical cleanup failed,
    // so the leftover directory is only an orphan.
    assert!(fixture.project.cache.get_entry(&head).is_none());
    assert!(blob.is_dir());

    fixture.reload();

    assert!(fixture.project.cache.get_entry(&head).is_none());
    assert!(blob.is_dir());
    assert!(!written(&fixture).contains("head.bin"));
  }

  #[test]
  fn removes_the_overrides_a_decode_declared_before_dropping_it() {
    let (mut fixture, source) = project();
    let head = vpath(&format!("{source}/head.bin"));

    fixture
      .project
      .add_decode(
        &vpath(&source),
        "std/generic/extract".to_owned(),
        &None,
        "head".to_owned(),
        artifacts(&["head.bin"]),
      )
      .unwrap();
    fixture.project.add_override(&head, false).unwrap();

    let workspace = fixture.project.workspace_path(&head);
    assert!(workspace.is_file());

    fixture
      .project
      .remove_decode(&vpath(&source), None, false)
      .unwrap();

    fixture.reload();

    assert!(fixture.project.manifest.overrides().is_empty());
    assert!(!fixture.project.manifest.is_overridden(&head));
    // Without `clean`, the workspace file is the operator's to keep.
    assert!(workspace.is_file());
  }

  #[test]
  fn cleans_the_workspace_files_of_a_removed_decodes_overrides() {
    let (mut fixture, source) = project();
    let head = vpath(&format!("{source}/head.bin"));

    fixture
      .project
      .add_decode(
        &vpath(&source),
        "std/generic/extract".to_owned(),
        &None,
        "head".to_owned(),
        artifacts(&["head.bin"]),
      )
      .unwrap();
    fixture.project.add_override(&head, false).unwrap();

    let workspace = fixture.project.workspace_path(&head);

    fixture
      .project
      .remove_decode(&vpath(&source), None, true)
      .unwrap();

    assert!(!workspace.exists());
  }
}
