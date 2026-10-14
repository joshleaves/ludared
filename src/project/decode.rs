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
      .add_decode(&self.codecs, path, codec_id, args, name, &artifacts)?;

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
}
