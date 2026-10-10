use log::*;

use crate::codecs::DecodedArtifact;
use crate::errors::app_error::AppError;
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
  pub(crate) fn add_decode(
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
      .add_decode(path, codec_id, args, name, &artifacts)?;

    for artifact in &artifacts {
      let artifact_path = path.join(&artifact.name)?;
      self.cache.add_entry(&artifact_path, &artifact.data)?;
    }
    self.cache.save()?;

    self.save_manifest()
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
    fixture.write_manifest(manifest(&source));
    fixture.reload();

    (fixture, source)
  }

  /// A manifest recording one source, holding no decodes.
  fn manifest(source: &str) -> String {
    format!(
      r#"{{
  "sources": {{ "{source}": {{ "sha256": "unverified", "size": null, "label": null }} }}
}}"#
    )
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
        "std/generic/extract_bytes".to_owned(),
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
        "std/generic/extract_bytes".to_owned(),
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
    assert_eq!(node.codec.args["offset"], 0);
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
          "std/generic/extract_bytes".to_owned(),
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
          "std/generic/extract_bytes".to_owned(),
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
          "std/generic/extract_bytes".to_owned(),
          &None,
          "head".to_owned(),
          artifacts(&["head.bin"]),
        )
        .is_err()
    );

    assert_eq!(written(&fixture), before);
  }
}
