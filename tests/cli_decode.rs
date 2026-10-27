//! End-to-end coverage of `ludared decode remove`.
//!
//! These tests drive the real binary against a throwaway project, so they cover
//! the parts a unit test cannot: argument wiring, the exit code a business case
//! produces, and what is left on disk after a refusal.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;

const SOURCE: &str = "game.sfc";
const SOURCE_BYTES: &[u8] = b"HELLOWORLD";
const EXTRACT: &str = "std/generic/extract";

/// A top-level artifact carrying a nested decode, and therefore not terminal.
const HEAD: &str = "game.sfc/head.bin";

/// The only decode attached to `HEAD`.
const TITLE: &str = "game.sfc/head.bin/TITLE.txt";

/// A throwaway project with one source and a decode tree of its own.
struct Project {
  dir: TempDir,
}

impl Project {
  /// A project whose cache has already been built from its manifest.
  fn unpacked() -> Self {
    let project = Self::unpacked_source_tree();

    project.ok(&["unpack"]);

    project
  }

  /// A project whose manifest records a decode tree that has not been replayed.
  fn unpacked_source_tree() -> Self {
    let project = Self::new();

    fs::create_dir_all(project.path("sources")).unwrap();
    fs::write(project.path("sources").join(SOURCE), SOURCE_BYTES).unwrap();
    fs::write(project.manifest_path(), project.decode_tree()).unwrap();

    project
  }

  fn new() -> Self {
    let dir = TempDir::new().unwrap();

    fs::write(
      dir.path().join("ludared.toml"),
      r#"
[project]
name = "decode-e2e"
manifest = "game.ludared"

[paths]
sources = "sources"
workspace = "workspace"
builds = "builds"
cache = "builds/cache"
"#,
    )
    .unwrap();

    Self { dir }
  }

  /// The manifest recording the decode tree every test works with.
  ///
  /// The source itself has two decodes, which is what makes a name mandatory
  /// there. `head.bin` carries the only decode below it, so that one does not
  /// need a name. `head.bin` is not terminal because `TITLE.txt` hangs off it.
  fn decode_tree(&self) -> String {
    let head = codec_args("head.bin", 0, 4);
    let title = codec_args("TITLE.txt", 0, 4);
    let tail = codec_args("tail.bin", 6, 4);

    format!(
      r#"{{
  "sources": {{
    "{SOURCE}": {{ "sha256": "0b21b7db59cd154904fac6336fa7d2be1bab38d632794f281549584068cdcb74", "size": null, "label": null }}
  }},
  "decodes": {{
    "{SOURCE}": [
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
    ]
  }}
}}"#
    )
  }

  fn run(&self, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ludared"))
      .current_dir(self.dir.path())
      .args(args)
      .output()
      .unwrap()
  }

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

  fn workspace_path(&self, vpath: &str) -> PathBuf {
    self.path("workspace").join(vpath)
  }

  fn manifest(&self) -> String {
    fs::read_to_string(self.manifest_path()).unwrap()
  }

  /// Whether any decode node of the manifest is named `name`.
  fn has_decode(&self, name: &str) -> bool {
    let manifest: serde_json::Value = serde_json::from_str(&self.manifest()).unwrap();

    fn find(name: &str, decodes: &[serde_json::Value]) -> bool {
      decodes.iter().any(|decode| {
        decode["name"] == name
          || decode["decodes"]
            .as_object()
            .into_iter()
            .flat_map(|nested| nested.values())
            .any(|children| find(name, children.as_array().unwrap()))
      })
    }

    find(name, manifest["decodes"][SOURCE].as_array().unwrap())
  }

  /// Every virtual path in the persisted cache index.
  fn cache_paths(&self) -> Vec<String> {
    let index = self.path("builds/cache/decodes/index.json");
    let index: serde_json::Value =
      serde_json::from_str(&fs::read_to_string(index).unwrap()).unwrap();

    let mut paths: Vec<String> = index["entries"]
      .as_object()
      .unwrap()
      .keys()
      .cloned()
      .collect();

    paths.sort();
    paths
  }

  fn override_path(&self, vpath: &str) {
    self.ok(&[
      "override",
      "add",
      vpath,
    ]);
  }
}

/// The extract codec's arguments, quoted the way the manifest records them.
fn codec_args(name: &str, offset: usize, length: usize) -> String {
  let args = serde_json::json!({ "name": name, "offset": offset, "length": length }).to_string();

  serde_json::Value::String(args).to_string()
}

/// Every file under `dir`, keyed by its path relative to `root`.
fn walk(dir: &Path, root: &Path) -> Vec<PathBuf> {
  let mut files: Vec<PathBuf> = fs::read_dir(dir)
    .unwrap()
    .flat_map(|entry| {
      let path = entry.unwrap().path();

      if path.is_dir() {
        walk(&path, root)
      } else {
        vec![path.strip_prefix(root).unwrap().to_path_buf()]
      }
    })
    .collect();

  files.sort();
  files
}

#[test]
fn removes_the_only_decode_of_an_artifact_without_a_name() {
  let project = Project::unpacked();

  assert_eq!(
    project.cache_paths(),
    [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
      "game.sfc/tail.bin",
    ]
  );

  assert_eq!(
    project.ok(&[
      "decode",
      "remove",
      HEAD
    ]),
    "✓ Removed decode 'TITLE' from game.sfc/head.bin\n"
  );

  assert!(!project.has_decode("TITLE"));
  assert!(project.has_decode("head"));
  assert!(project.has_decode("tail"));

  // Only the removed decode's output leaves the cache index.
  assert_eq!(
    project.cache_paths(),
    [
      "game.sfc/head.bin",
      "game.sfc/tail.bin",
    ]
  );
}

#[test]
fn requires_a_name_when_several_decodes_share_an_artifact() {
  let project = Project::unpacked();
  let manifest = project.manifest();
  let cache = project.cache_paths();

  let stderr = project.fails(&[
    "decode",
    "remove",
    SOURCE,
  ]);

  assert!(
    stderr.contains("Multiple decodes"),
    "unexpected error: {stderr}"
  );

  // Nothing is selected, so neither the manifest nor the cache is touched.
  assert_eq!(project.manifest(), manifest);
  assert_eq!(project.cache_paths(), cache);

  assert_eq!(
    project.ok(&[
      "decode",
      "remove",
      SOURCE,
      "tail",
    ]),
    "✓ Removed decode 'tail' from game.sfc\n"
  );

  assert!(project.has_decode("head"));
  assert!(project.has_decode("TITLE"));
  assert!(!project.has_decode("tail"));
  assert_eq!(
    project.cache_paths(),
    [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
    ]
  );
}

#[test]
fn refuses_a_decode_with_child_decodes() {
  let project = Project::unpacked();
  let manifest = project.manifest();
  let cache = project.cache_paths();

  let stderr = project.fails(&[
    "decode",
    "remove",
    SOURCE,
    "head",
  ]);

  assert!(
    stderr.contains("it has children"),
    "unexpected error: {stderr}"
  );
  assert_eq!(project.manifest(), manifest);
  assert_eq!(project.cache_paths(), cache);
}

#[test]
fn refuses_a_name_no_decode_on_the_artifact_has() {
  let project = Project::unpacked();
  let manifest = project.manifest();

  let stderr = project.fails(&[
    "decode",
    "remove",
    SOURCE,
    "missing",
  ]);

  assert!(
    stderr.contains("Decode 'missing' not found"),
    "unexpected error: {stderr}"
  );
  assert_eq!(project.manifest(), manifest);
}

#[test]
fn lifts_the_overrides_the_removed_decode_declared() {
  let project = Project::unpacked();

  project.override_path(TITLE);
  assert!(project.workspace_path(TITLE).is_file());

  project.ok(&[
    "decode",
    "remove",
    HEAD,
  ]);

  // The declaration goes with the decode it belonged to, while the file is the
  // operator's unless `--clean` says otherwise.
  assert!(!project.manifest().contains("\"overrides\""));
  assert!(project.workspace_path(TITLE).is_file());
  assert!(!project.has_decode("TITLE"));
}

#[test]
fn cleans_the_workspace_files_of_the_overrides_it_lifts() {
  let project = Project::unpacked();

  project.override_path(TITLE);
  assert!(project.workspace_path(TITLE).is_file());

  project.ok(&[
    "decode",
    "remove",
    HEAD,
    "--clean",
  ]);

  assert!(!project.workspace_path(TITLE).exists());
  assert!(!project.has_decode("TITLE"));
}

#[test]
fn removes_the_decode_even_when_a_workspace_cleanup_fails() {
  let project = Project::unpacked();

  project.override_path(TITLE);

  // A directory where the workspace file belongs cannot be deleted.
  let workspace = project.workspace_path(TITLE);
  fs::remove_file(&workspace).unwrap();
  fs::create_dir(&workspace).unwrap();

  project.ok(&[
    "decode",
    "remove",
    HEAD,
    "--clean",
  ]);

  // The manifest and cache are persisted before physical cleanup starts, so a
  // failure to delete the workspace file leaves an orphan but does not undo the
  // removal or leave metadata pointing at missing data.
  assert!(!project.has_decode("TITLE"));
  assert!(!project.manifest().contains("\"overrides\""));
  assert!(!project.cache_paths().iter().any(|path| path == TITLE));
  assert!(workspace.is_dir());
}

#[test]
fn keeps_a_blob_another_artifact_still_references() {
  let project = Project::unpacked();
  let before = walk(
    &project.path("builds/cache/decodes"),
    &project.path("builds/cache/decodes"),
  );

  project.ok(&[
    "decode",
    "remove",
    HEAD,
  ]);

  // `head.bin` and `TITLE.txt` hold the same bytes, so removing the decode that
  // produced the latter leaves the shared blob for the former.
  assert_eq!(
    walk(
      &project.path("builds/cache/decodes"),
      &project.path("builds/cache/decodes")
    ),
    before
  );
}
