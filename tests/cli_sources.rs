//! End-to-end coverage of `ludared sources remove`.
//!
//! These tests drive the real binary against a throwaway project, so they cover
//! what the unit tests cannot: that removing a source also drops the decode tree
//! it owned, that the decode cache forgets the artifacts under it, and that
//! neither reaches into a neighbouring source.
//!
//! The regression they guard against is a source being removed from `sources`
//! while its bucket stayed in `decodes`, which left `artifacts list` reporting
//! paths no resolver could reach.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use sha2::Digest as _;
use tempfile::TempDir;

const SOURCE: &str = "game.sfc";

/// A source name that shares its text with `SOURCE`, but is a component away
/// from being under it.
const NEIGHBOUR: &str = "game.sfc.bak";

const SOURCE_BYTES: &[u8] = b"HELLOWORLD";
const EXTRACT: &str = "std/generic/extract";

/// A top-level artifact.
const HEAD: &str = "game.sfc/head.bin";

/// An artifact produced by a decode nested under `HEAD`.
const TITLE: &str = "game.sfc/head.bin/TITLE.txt";

/// A sibling top-level artifact.
const TAIL: &str = "game.sfc/tail.bin";

/// A throwaway project holding the given sources, each with the same decode
/// tree and the same bytes.
struct Project {
  dir: TempDir,
}

impl Project {
  /// A project with one source and its decode tree recorded, cache not replayed.
  fn with_tree() -> Self {
    Self::with_sources(&[SOURCE])
  }

  /// A project whose manifest records `sources`, each with the decode tree
  /// [`decode_nodes`] describes.
  fn with_sources(sources: &[&str]) -> Self {
    let dir = TempDir::new().unwrap();

    fs::write(
      dir.path().join("ludared.toml"),
      r#"
[project]
name = "sources-e2e"
manifest = "game.ludared"

[paths]
sources = "sources"
workspace = "workspace"
builds = "builds"
cache = "builds/cache"
"#,
    )
    .unwrap();

    let project = Self { dir };

    fs::create_dir_all(project.path("sources")).unwrap();
    for source in sources {
      fs::write(project.path("sources").join(source), SOURCE_BYTES).unwrap();
    }
    fs::write(project.manifest_path(), manifest(sources)).unwrap();

    project
  }

  fn run(&self, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ludared"))
      .current_dir(self.dir.path())
      .args(args)
      .output()
      .unwrap()
  }

  /// Runs a command that is expected to succeed, and returns what it printed.
  fn ok(&self, args: &[&str]) -> String {
    let output = self.run(args);

    assert!(
      output.status.success(),
      "`ludared {}` failed: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).unwrap()
  }

  /// Runs a command that is expected to fail, and returns what it reported.
  fn fails(&self, args: &[&str]) -> String {
    let output = self.run(args);

    assert!(
      !output.status.success(),
      "`ludared {}` unexpectedly succeeded",
      args.join(" ")
    );

    String::from_utf8(output.stderr).unwrap()
  }

  fn path(&self, relative: &str) -> PathBuf {
    self.dir.path().join(relative)
  }

  fn manifest_path(&self) -> PathBuf {
    self.path("game.ludared")
  }

  /// The manifest as it stands on disk.
  fn manifest(&self) -> serde_json::Value {
    self.json("game.ludared")
  }

  fn json(&self, relative: &str) -> serde_json::Value {
    let content = fs::read_to_string(self.path(relative)).unwrap();

    serde_json::from_str(&content).unwrap_or_else(|err| panic!("'{relative}' is not JSON: {err}"))
  }

  /// The cache blob backing `artifact`, located through the CLI itself.
  ///
  /// The configured cache path is relative to the project, so the blob the
  /// command prints is resolved against the project directory. An absolute path
  /// is left as it is by `join`.
  fn cache_blob(&self, artifact: &str) -> PathBuf {
    self.path(
      self
        .ok(&[
          "cache",
          "path",
          artifact,
        ])
        .trim(),
    )
  }
}

/// A manifest recording `sources` and the same decode tree under each.
fn manifest(sources: &[&str]) -> String {
  let source_entries = sources
    .iter()
    .map(|source| {
      format!(
        r#""{source}": {{ "sha256": "{}", "size": null, "label": null }}"#,
        sha256_hex(SOURCE_BYTES)
      )
    })
    .collect::<Vec<_>>()
    .join(",\n    ");

  let decode_entries = sources
    .iter()
    .map(|source| format!(r#""{source}": {}"#, decode_nodes()))
    .collect::<Vec<_>>()
    .join(",\n    ");

  format!(
    r#"{{
  "sources": {{
    {source_entries}
  }},
  "decodes": {{
    {decode_entries}
  }}
}}"#
  )
}

/// Two decodes on a source, one of them nested under `head.bin`.
fn decode_nodes() -> String {
  let head = codec_args("head.bin", 0, 4);
  let title = codec_args("TITLE.txt", 0, 4);
  let tail = codec_args("tail.bin", 6, 4);

  format!(
    r#"[
      {{
        "name": "head",
        "codec": {{ "id": "{EXTRACT}", "version": 1, "args": {head} }},
        "outputs": [ "head.bin" ],
        "decodes": {{
          "head.bin": [
            {{
              "name": "TITLE",
              "codec": {{ "id": "{EXTRACT}", "version": 1, "args": {title} }},
              "outputs": [ "TITLE.txt" ],
              "decodes": {{}}
            }}
          ]
        }}
      }},
      {{
        "name": "tail",
        "codec": {{ "id": "{EXTRACT}", "version": 1, "args": {tail} }},
        "outputs": [ "tail.bin" ],
        "decodes": {{}}
      }}
    ]"#
  )
}

/// The extract codec's arguments, quoted the way the manifest records them.
fn codec_args(name: &str, offset: usize, length: usize) -> String {
  let args = serde_json::json!({ "name": name, "offset": offset, "length": length }).to_string();

  serde_json::Value::String(args).to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
  sha2::Sha256::digest(bytes)
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect()
}

fn lines(output: &str) -> Vec<&str> {
  output
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .collect()
}

#[test]
fn removing_a_source_drops_the_decode_tree_it_owned() {
  let project = Project::with_tree();
  project.ok(&["unpack"]);

  assert_eq!(
    lines(&project.ok(&["artifacts", "list"])),
    [
      SOURCE,
      HEAD,
      TITLE,
      TAIL,
    ]
  );

  project.ok(&[
    "sources",
    "remove",
    SOURCE,
  ]);

  // The source and everything decoded from it are gone from the manifest, not
  // just hidden from one listing.
  let manifest = project.manifest();
  assert!(manifest["sources"].get(SOURCE).is_none());
  assert!(
    manifest["decodes"].get(SOURCE).is_none(),
    "the decode tree should go with its source: {manifest}"
  );
  assert!(manifest["decodes"].as_object().unwrap().is_empty());

  assert_eq!(project.ok(&["artifacts", "list"]), "");
  assert_eq!(project.ok(&["sources", "list"]), "");

  // Nothing recorded the removed artifacts, so they are no longer addressable.
  let stderr = project.fails(&[
    "decode",
    "list",
    HEAD,
  ]);
  assert!(
    stderr.contains("Could not resolve virtual path"),
    "{stderr}"
  );
}

#[test]
fn removing_a_source_forgets_its_artifacts_in_the_decode_cache() {
  let project = Project::with_tree();
  project.ok(&["unpack"]);

  let blobs = [HEAD, TITLE, TAIL].map(|artifact| project.cache_blob(artifact));

  for blob in &blobs {
    assert!(blob.is_file(), "{} should be cached", blob.display());
  }

  project.ok(&[
    "sources",
    "remove",
    SOURCE,
  ]);

  // Every artifact the source owned left the index.
  for artifact in [HEAD, TITLE, TAIL] {
    let stderr = project.fails(&[
      "cache",
      "path",
      artifact,
    ]);
    assert!(stderr.contains("not indexed"), "{stderr}");
  }

  // With nothing else naming them, the blobs followed the index entries.
  for blob in &blobs {
    assert!(
      !blob.exists(),
      "{} should have been deleted",
      blob.display()
    );
  }

  let index = project.json("builds/cache/decodes/index.json");
  assert_eq!(index["entries"].as_object().unwrap().len(), 0);
}

#[test]
fn removing_a_source_leaves_a_neighbouring_source_alone() {
  let project = Project::with_sources(&[SOURCE, NEIGHBOUR]);
  project.ok(&["unpack"]);

  project.ok(&[
    "sources",
    "remove",
    SOURCE,
  ]);

  // `game.sfc.bak` shares `game.sfc` as plain text but not as a path, so it is
  // not under the source that was removed.
  let manifest = project.manifest();
  assert!(manifest["sources"].get(SOURCE).is_none());
  assert!(manifest["sources"].get(NEIGHBOUR).is_some());
  assert!(manifest["decodes"].get(SOURCE).is_none());
  assert!(
    manifest["decodes"].get(NEIGHBOUR).is_some(),
    "the neighbour's decode tree should be untouched: {manifest}"
  );

  let neighbouring = [
    NEIGHBOUR,
    "game.sfc.bak/head.bin",
    "game.sfc.bak/head.bin/TITLE.txt",
    "game.sfc.bak/tail.bin",
  ];
  assert_eq!(
    lines(&project.ok(&["artifacts", "list"])),
    neighbouring.as_slice()
  );

  // Its cache entries still resolve; the removed source's do not.
  let blob = project.cache_blob("game.sfc.bak/head.bin");
  assert!(blob.is_file());
  assert!(
    project
      .fails(&[
        "cache",
        "path",
        HEAD
      ])
      .contains("not indexed")
  );
}

#[test]
fn readding_a_source_does_not_resurrect_the_decode_tree_it_had() {
  let project = Project::with_tree();
  project.ok(&["unpack"]);

  project.ok(&[
    "sources",
    "remove",
    SOURCE,
  ]);

  // The file was never touched; only its declaration and its tree went.
  project.ok(&[
    "sources",
    "add",
    SOURCE,
  ]);

  assert_eq!(lines(&project.ok(&["artifacts", "list"])), [SOURCE]);
  // A source with no decodes lists nothing, rather than the tree it used to own.
  assert_eq!(
    project.ok(&[
      "decode",
      "list",
      SOURCE
    ]),
    ""
  );
  assert!(project.manifest()["decodes"].get(SOURCE).is_none());
}
