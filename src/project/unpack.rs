use log::*;
use std::collections::HashSet;

use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::registry::CodecRegistry;
use crate::errors::app_error::AppError;
use crate::manifest::decode_node::DecodeNode;
use crate::manifest::errors::ManifestError;
use crate::project::cache::Cache;
use crate::virtual_path::VirtualPath;
use crate::virtual_path::errors::VirtualPathError;

use super::Project;

/// Tally of the work performed by [`Project::unpack`].
#[derive(Debug, Default)]
pub(crate) struct UnpackReport {
  /// Number of decode nodes replayed.
  pub(crate) decodes: usize,

  /// Number of artifacts written back into the cache.
  pub(crate) artifacts: usize,
}

impl Project {
  /// Recreates the decode cache from the project sources and the manifest.
  ///
  /// Every source registered in the manifest is read from disk, and the decode
  /// tree recorded for it is replayed depth-first. Each decode node is executed
  /// again with the codec and arguments it recorded, and its artifacts are
  /// written back into the cache under the same virtual paths they were first
  /// given. The decodes attached to those outputs are then replayed with the
  /// freshly produced bytes, so an artifact never has to be read back out of the
  /// cache it is being written into.
  ///
  /// The cache index is rebuilt rather than merged, so once the tree has been
  /// replayed the index describes exactly the artifacts the manifest records and
  /// nothing an earlier decode tree left behind. Content-addressed blobs for
  /// dropped entries are left on disk.
  ///
  /// The index is only written once the whole tree has been replayed
  /// successfully. The manifest is only ever read: it is never modified and
  /// never written back. If replay fails, the previously persisted cache index
  /// remains untouched; a later unpack rebuilds the in-memory index from scratch.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if:
  /// - a source recorded in the manifest cannot be read;
  /// - a decode node records a codec that is not available;
  /// - a codec fails to decode its input;
  /// - a codec no longer produces the outputs recorded for its decode node;
  /// - the cache or its index cannot be written.
  pub(crate) fn unpack(&mut self) -> Result<UnpackReport, AppError> {
    debug!("Unpacking decode cache of project '{}'", self.name());
    let mut report = UnpackReport::default();

    self.cache.clear_index();

    for source_name in self.manifest.sources.keys() {
      let Some(source) = source_name.to_str() else {
        return Err(VirtualPathError::NonUtf8Path(source_name.clone()).into());
      };

      let decodes = self
        .manifest
        .decodes
        .get(source)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
      if decodes.is_empty() {
        trace!("No decodes recorded for source '{source}'");
        continue;
      }

      let vpath = VirtualPath::new(source)?;
      let source_path = self.source_path(source_name);
      let data =
        std::fs::read(&source_path).map_err(|err| AppError::SourceFileIo(source_path, err))?;

      trace!(
        "Replaying {} decode(s) for source '{source}'",
        decodes.len()
      );
      unpack_decodes(&mut self.cache, &vpath, &data, decodes, &mut report)?;
    }

    debug!(
      "Replayed {} decode(s), restored {} artifact(s)",
      report.decodes, report.artifacts
    );
    self.cache.save()?;

    Ok(report)
  }
}

/// Replays every decode node attached to an artifact, in manifest order.
fn unpack_decodes(
  cache: &mut Cache,
  path: &VirtualPath,
  input: &[u8],
  decodes: &[DecodeNode],
  report: &mut UnpackReport,
) -> Result<(), AppError> {
  for node in decodes {
    unpack_decode(cache, path, input, node, report)?;
  }

  Ok(())
}

/// Replays a single decode node and, in turn, every decode attached to its
/// outputs.
fn unpack_decode(
  cache: &mut Cache,
  path: &VirtualPath,
  input: &[u8],
  node: &DecodeNode,
  report: &mut UnpackReport,
) -> Result<(), AppError> {
  debug!(
    "Replaying decode '{}' ({}) on '{path}'",
    node.name, node.codec.id
  );

  let codec = CodecRegistry::get(&node.codec.id)
    .ok_or_else(|| AppError::CodecUnavailable(node.codec.id.clone()))?;

  let args = recorded_args(&node.codec.args)?;
  let artifacts = codec.decode(input, args.as_deref())?;

  verify_outputs(path, node, &artifacts)?;

  for artifact in &artifacts {
    let artifact_path = path.join(&artifact.name)?;
    debug!("  restored '{artifact_path}'");

    cache.add_entry(&artifact_path, &artifact.data)?;
    report.artifacts += 1;

    let nested = node
      .decodes
      .get(&artifact.name)
      .map(Vec::as_slice)
      .unwrap_or(&[]);
    unpack_decodes(cache, &artifact_path, &artifact.data, nested, report)?;
  }

  report.decodes += 1;

  Ok(())
}

/// Renders the arguments recorded in the manifest back into the JSON string the
/// codec API expects.
///
/// Absent arguments are recorded as `null`; codecs already treat a missing
/// argument list as an empty object, so those are handed over as `None`.
fn recorded_args(args: &serde_json::Value) -> Result<Option<String>, AppError> {
  match args {
    serde_json::Value::Null => Ok(None),
    args => Ok(Some(
      serde_json::to_string(args).map_err(CodecError::JsonArgs)?,
    )),
  }
}

/// Verifies that a replayed decode node produced exactly the outputs recorded
/// for it in the manifest.
///
/// The manifest is the source of truth for a decode recipe, so an output that
/// disappeared or was renamed by its codec means the recorded recipe can no
/// longer be replayed and the cache would not match the manifest.
///
/// A codec handing back the same name twice is rejected first: it breaks the
/// invariant `decode add` already upholds, and the set comparison below cannot
/// see it.
fn verify_outputs(
  path: &VirtualPath,
  node: &DecodeNode,
  artifacts: &[DecodedArtifact],
) -> Result<(), AppError> {
  let mut produced: HashSet<&str> = HashSet::with_capacity(artifacts.len());
  for artifact in artifacts {
    if !produced.insert(artifact.name.as_str()) {
      return Err(CodecError::DuplicateArtifact(artifact.name.clone()).into());
    }
  }

  if let Some(output) = node
    .outputs
    .iter()
    .find(|output| !produced.contains(output.as_str()))
  {
    return Err(
      ManifestError::MissingOutput(node.name.clone(), path.to_string(), output.clone()).into(),
    );
  }

  if let Some(artifact) = artifacts
    .iter()
    .find(|artifact| !node.outputs.contains(&artifact.name))
  {
    return Err(
      ManifestError::UnexpectedOutput(node.name.clone(), path.to_string(), artifact.name.clone())
        .into(),
    );
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::fixtures::lorom;
  use crate::testing::fixtures::project::ProjectFixture;

  const EXTRACT: &str = "std/generic/extract_bytes";

  /// A project with one registered source and a manifest written by hand, since
  /// the decode tree unpack replays is exactly what is under test.
  ///
  /// Cache roots are resolved against the working directory rather than the
  /// project root, so every fixture shares one decode cache. The source name is
  /// randomised to keep each test's virtual paths to itself.
  fn fixture(source: &[u8], decodes: &str) -> (ProjectFixture, String) {
    let (source_name, source_path) = ProjectFixture::random_source_name();
    let mut fixture = ProjectFixture::new();

    fixture.register_source_file(&source_path, source);
    fixture.write_manifest(manifest(&source_name, decodes));
    fixture.reload();

    (fixture, source_name)
  }

  /// A manifest recording a single source and the decode nodes attached to it.
  fn manifest(source: &str, decodes: &str) -> String {
    format!(
      r#"{{
  "sources": {{
    "{source}": {{ "sha256": "unverified", "size": null, "label": null }}
  }},
  "decodes": {{
    "{source}": {decodes}
  }}
}}"#
    )
  }

  fn extract(name: &str, target: &str, offset: usize, length: usize, decodes: &str) -> String {
    format!(
      r#"{{
      "name": "{name}",
      "codec": {{
        "id": "{EXTRACT}",
        "version": 1,
        "args": {{ "target": "{target}", "offset": {offset}, "length": {length} }}
      }},
      "outputs": [ "{target}" ],
      "decodes": {decodes}
    }}"#
    )
  }

  /// Reads back the bytes of an artifact cached under `source`.
  fn cached(fixture: &ProjectFixture, source: &str, artifact: &str) -> Vec<u8> {
    let vpath = VirtualPath::new(&format!("{source}/{artifact}")).unwrap();
    let entry = fixture
      .project
      .cache
      .get_entry(&vpath)
      .unwrap_or_else(|| panic!("'{source}/{artifact}' missing from the decode cache"));

    std::fs::read(entry).unwrap()
  }

  #[test]
  fn restores_artifacts_from_manifest() {
    let (mut fixture, source) = fixture(
      b"HELLOWORLD",
      &format!("[{}]", extract("head", "head.bin", 0, 4, "{}")),
    );

    let report = fixture.project.unpack().unwrap();

    assert_eq!(report.decodes, 1);
    assert_eq!(report.artifacts, 1);
    assert_eq!(cached(&fixture, &source, "head.bin"), b"HELL");
  }

  #[test]
  fn saves_the_cache_index() {
    let (mut fixture, source) = fixture(
      b"HELLOWORLD",
      &format!("[{}]", extract("head", "head.bin", 0, 4, "{}")),
    );

    fixture.project.unpack().unwrap();
    fixture.reload();

    assert_eq!(cached(&fixture, &source, "head.bin"), b"HELL");
  }

  #[test]
  fn restores_artifacts_of_nested_decodes() {
    let inner = extract("inner", "inner.bin", 0, 6, "{}");
    let nested = format!("[{}]", extract("tail", "TAIL.txt", 1, 3, "{}"));
    let outer = extract(
      "outer",
      "outer.bin",
      0,
      10,
      &format!(r#"{{ "outer.bin": {nested} }}"#),
    );
    let (mut fixture, source) = fixture(b"HELLOWORLD", &format!("[{inner}, {outer}]"));

    let report = fixture.project.unpack().unwrap();

    assert_eq!(report.decodes, 3);
    assert_eq!(report.artifacts, 3);
    assert_eq!(cached(&fixture, &source, "inner.bin"), b"HELLOW");
    assert_eq!(cached(&fixture, &source, "outer.bin"), b"HELLOWORLD");
    assert_eq!(cached(&fixture, &source, "outer.bin/TAIL.txt"), b"ELL");
  }

  #[test]
  fn replays_decodes_of_every_source() {
    let (first, first_path) = ProjectFixture::random_source_name();
    let (second, second_path) = ProjectFixture::random_source_name();
    let mut fixture = ProjectFixture::new();

    fixture.register_source_file(&first_path, b"HELLOWORLD");
    fixture.register_source_file(&second_path, b"GOODBYE");
    fixture.write_manifest(format!(
      r#"{{
  "sources": {{
    "{first}": {{ "sha256": "unverified", "size": null, "label": null }},
    "{second}": {{ "sha256": "unverified", "size": null, "label": null }}
  }},
  "decodes": {{
    "{first}": [{}],
    "{second}": [{}]
  }}
}}"#,
      extract("head", "head.bin", 0, 5, "{}"),
      extract("tail", "tail.bin", 4, 3, "{}"),
    ));
    fixture.reload();

    fixture.project.unpack().unwrap();

    assert_eq!(cached(&fixture, &first, "head.bin"), b"HELLO");
    assert_eq!(cached(&fixture, &second, "tail.bin"), b"BYE");
  }

  #[test]
  fn leaves_sources_without_decodes_alone() {
    let (mut fixture, source) = fixture(b"HELLOWORLD", "[]");

    let report = fixture.project.unpack().unwrap();

    assert_eq!(report.decodes, 0);
    assert_eq!(report.artifacts, 0);
    assert!(
      !fixture
        .project
        .cache
        .entries()
        .any(|entry| entry.starts_with(&source))
    );
  }

  #[test]
  fn drops_index_entries_a_previous_decode_tree_left_behind() {
    let (mut fixture, source) = fixture(
      b"HELLOWORLD",
      &format!("[{}]", extract("head", "head.bin", 0, 4, "{}")),
    );

    fixture.project.unpack().unwrap();
    assert_eq!(cached(&fixture, &source, "head.bin"), b"HELL");

    // The decode tree moves on: `head.bin` is no longer recorded anywhere.
    let decodes = format!("[{}]", extract("tail", "tail.bin", 6, 4, "{}"));
    fixture.write_manifest(manifest(&source, &decodes));
    fixture.reload();

    fixture.project.unpack().unwrap();

    assert_eq!(cached(&fixture, &source, "tail.bin"), b"ORLD");
    assert_eq!(
      fixture
        .project
        .cache
        .entries()
        .filter(|entry| entry.starts_with(&source))
        .count(),
      1,
      "'head.bin' should no longer be in the index"
    );

    // The index is only written once the replay succeeds, so the stale entry is
    // gone from disk too, while its blob is left where it was.
    fixture.reload();
    assert!(
      fixture
        .project
        .cache
        .get_entry(&VirtualPath::new(&format!("{source}/head.bin")).unwrap())
        .is_none()
    );
    assert_eq!(cached(&fixture, &source, "tail.bin"), b"ORLD");
  }

  #[test]
  fn rejects_an_unrecorded_duplicate_artifact_name() {
    let vpath = VirtualPath::new("game.sfc").unwrap();
    let node = DecodeNode {
      name: "head".to_owned(),
      codec: crate::manifest::decode_node::CodecNode {
        id: EXTRACT.to_owned(),
        version: 1,
        args: Default::default(),
      },
      outputs: vec!["head.bin".to_owned()],
      decodes: Default::default(),
    };
    let artifacts = [
      DecodedArtifact {
        name: "head.bin".to_owned(),
        data: b"HELL".to_vec(),
      },
      DecodedArtifact {
        name: "head.bin".to_owned(),
        data: b"WORL".to_vec(),
      },
    ];

    // No builtin codec can return the same name twice, so `verify_outputs` is
    // exercised directly on the artifacts it would have been handed.
    match verify_outputs(&vpath, &node, &artifacts).unwrap_err() {
      AppError::CodecError(CodecError::DuplicateArtifact(name)) => assert_eq!(name, "head.bin"),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn does_not_modify_the_manifest() {
    let decodes = format!("[{}]", extract("head", "head.bin", 0, 4, "{}"));
    let (mut fixture, source) = fixture(b"HELLOWORLD", &decodes);
    let written = manifest(&source, &decodes);

    fixture.project.unpack().unwrap();

    let path = fixture
      .project
      .root
      .join(&fixture.project.configuration.project.manifest);
    assert_eq!(std::fs::read_to_string(path).unwrap(), written);
  }

  #[test]
  fn fails_if_codec_is_unavailable() {
    let decodes = r#"[{
      "name": "gone",
      "codec": { "id": "std/gone/missing", "version": 1, "args": {} },
      "outputs": [ "gone.bin" ],
      "decodes": {}
    }]"#;
    let (mut fixture, _) = fixture(b"HELLOWORLD", decodes);

    match fixture.project.unpack().unwrap_err() {
      AppError::CodecUnavailable(id) => assert_eq!(id, "std/gone/missing"),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn fails_if_decoding_fails() {
    let (mut fixture, _source) = fixture(
      b"HELLOWORLD",
      &format!("[{}]", extract("oob", "oob.bin", 8, 8, "{}")),
    );

    match fixture.project.unpack().unwrap_err() {
      AppError::CodecError(CodecError::Message(message)) => {
        assert!(
          message.contains("Out of bounds"),
          "unexpected message: {message}"
        );
      }
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn fails_if_recorded_output_was_not_produced() {
    // The codec still produces `head.bin`, but the manifest renamed it.
    let node = r#"{
      "name": "head",
      "codec": {
        "id": "std/generic/extract_bytes",
        "version": 1,
        "args": { "target": "head.bin", "offset": 0, "length": 4 }
      },
      "outputs": [ "renamed.bin" ],
      "decodes": {}
    }"#;
    let (mut fixture, source) = fixture(b"HELLOWORLD", &format!("[{node}]"));

    match fixture.project.unpack().unwrap_err() {
      AppError::ManifestError(ManifestError::MissingOutput(name, path, output)) => {
        assert_eq!(name, "head");
        assert_eq!(path, source);
        assert_eq!(output, "renamed.bin");
      }
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn fails_if_codec_produces_an_unrecorded_output() {
    // The codec still produces `head.bin`, but the manifest records nothing.
    let node = r#"{
      "name": "head",
      "codec": {
        "id": "std/generic/extract_bytes",
        "version": 1,
        "args": { "target": "head.bin", "offset": 0, "length": 4 }
      },
      "outputs": [],
      "decodes": {}
    }"#;
    let (mut fixture, source) = fixture(b"HELLOWORLD", &format!("[{node}]"));

    match fixture.project.unpack().unwrap_err() {
      AppError::ManifestError(ManifestError::UnexpectedOutput(name, path, output)) => {
        assert_eq!(name, "head");
        assert_eq!(path, source);
        assert_eq!(output, "head.bin");
      }
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn restores_a_real_lorom_and_its_nested_decode() {
    let (mut fixture, source) = lorom::project();

    let report = fixture.project.unpack().unwrap();

    assert_eq!(report.decodes, 2);
    assert_eq!(report.artifacts, 3);
    assert_eq!(
      cached(&fixture, &source, "rom_bank_00.bin"),
      &lorom::SOURCE[..0x8000]
    );
    assert_eq!(
      cached(&fixture, &source, "rom_bank_01.bin"),
      &lorom::SOURCE[0x8000..]
    );
    assert_eq!(
      cached(&fixture, &source, lorom::TITLE_ARTIFACT),
      lorom::TITLE
    );
  }
}
