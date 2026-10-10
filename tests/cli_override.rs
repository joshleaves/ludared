//! End-to-end coverage of `ludared override`.
//!
//! These tests drive the real binary against a throwaway project, so they cover
//! the parts a unit test cannot: argument wiring, the exit code a business case
//! produces, and the wording an operator is pointed at by a failure. Each test
//! owns its project directory and runs the binary with that directory as its
//! working directory, which is what the CLI resolves a project from.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const SOURCE: &str = "game.sfc";
const SOURCE_BYTES: &[u8] = b"HELLOWORLD";
const EXTRACT: &str = "std/generic/extract_bytes";

/// A top-level leaf artifact.
const TAIL: &str = "game.sfc/tail.bin";

/// An artifact carrying a nested decode, which is therefore not a leaf.
const HEAD: &str = "game.sfc/head.bin";

/// A nested leaf artifact.
const TITLE: &str = "game.sfc/head.bin/TITLE.txt";

/// A throwaway project with one source and a decode tree of its own.
struct Project {
  dir: TempDir,
}

impl Project {
  /// A project whose cache has already been built from its manifest.
  fn unpacked() -> Self {
    let project = Self::unpacked_source_tree();

    project.run(&["unpack"]);

    project
  }

  /// A project whose manifest records a decode tree that has not been replayed.
  fn unpacked_source_tree() -> Self {
    let project = Self::new();

    fs::create_dir_all(project.dir.path().join("sources")).unwrap();
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
name = "override-e2e"
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
  /// `extract_bytes` produces a single artifact per decode node, so the tree is
  /// two decodes on the source with one decode nested under `head.bin`. That
  /// gives the three shapes the commands have to tell apart: a top-level leaf, a
  /// nested leaf, and an artifact with decodes of its own.
  fn decode_tree(&self) -> String {
    format!(
      r#"{{
  "sources": {{
    "{SOURCE}": {{ "sha256": "unverified", "size": null, "label": null }}
  }},
  "decodes": {{
    "{SOURCE}": [
      {{
        "name": "head",
        "codec": {{ "id": "{EXTRACT}", "version": 1, "args": {{ "target": "head.bin", "offset": 0, "length": 4 }} }},
        "outputs": [ "head.bin" ],
        "decodes": {{
          "head.bin": [
            {{
              "name": "TITLE",
              "codec": {{ "id": "{EXTRACT}", "version": 1, "args": {{ "target": "TITLE.txt", "offset": 0, "length": 4 }} }},
              "outputs": [ "TITLE.txt" ],
              "decodes": {{}}
            }}
          ]
        }}
      }},
      {{
        "name": "tail",
        "codec": {{ "id": "{EXTRACT}", "version": 1, "args": {{ "target": "tail.bin", "offset": 6, "length": 4 }} }},
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

  /// Declares `vpath` as an override, and fails the test if that does not work.
  fn override_path(&self, vpath: &str) {
    self.ok(&[
      "override",
      "add",
      vpath,
    ]);
  }

  fn path(&self, relative: &str) -> PathBuf {
    self.dir.path().join(relative)
  }

  fn manifest_path(&self) -> PathBuf {
    self.path("game.ludared")
  }

  /// The workspace file backing an artifact, at the fixed physical path the
  /// workspace mirrors the virtual file system with.
  fn workspace_path(&self, vpath: &str) -> PathBuf {
    self.path("workspace").join(vpath)
  }

  fn manifest(&self) -> String {
    fs::read_to_string(self.manifest_path()).unwrap()
  }

  /// Whether the decode node named `node` declares `output` as an override.
  ///
  /// The manifest is written with `serde_json`, which lays arrays out over one
  /// line per element, so the declaration is read back structurally rather than
  /// matched as text. The whole decode tree is searched, since a node may hang
  /// off an output of another.
  fn declares_override(&self, node: &str, output: &str) -> bool {
    let manifest: serde_json::Value =
      serde_json::from_str(&self.manifest()).expect("manifest should be valid JSON");

    fn find(node: &str, decodes: &[serde_json::Value]) -> Option<serde_json::Value> {
      for decode in decodes {
        if decode["name"] == node {
          return Some(decode.clone());
        }

        for nested in decode["decodes"]
          .as_object()
          .into_iter()
          .flat_map(|d| d.values())
        {
          if let Some(found) = find(node, nested.as_array().unwrap()) {
            return Some(found);
          }
        }
      }

      None
    }

    let decodes = manifest["decodes"][SOURCE]
      .as_array()
      .expect("decodes should be an array");
    let node = find(node, decodes).unwrap_or_else(|| panic!("no decode node named '{node}'"));

    node["overrides"]
      .as_array()
      // A decode node declaring none leaves the field out entirely.
      .map_or([].as_slice(), Vec::as_slice)
      .iter()
      .any(|declared| declared == output)
  }

  /// The number of decode nodes of the manifest carrying an `overrides` field.
  fn override_fields(&self) -> usize {
    fn count(node: &serde_json::Value) -> usize {
      let mut total = usize::from(node.get("overrides").is_some());

      for nested in node["decodes"]
        .as_object()
        .into_iter()
        .flat_map(|d| d.values())
      {
        for child in nested.as_array().unwrap() {
          total += count(child);
        }
      }

      total
    }

    let manifest: serde_json::Value = serde_json::from_str(&self.manifest()).unwrap();

    manifest["decodes"][SOURCE]
      .as_array()
      .unwrap()
      .iter()
      .map(count)
      .sum()
  }

  /// Writes a file into the workspace, creating the directories it needs.
  fn write_workspace(&self, vpath: &str, bytes: &[u8]) {
    let path = self.workspace_path(vpath);

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
  }

  /// Every file of the decode cache, keyed by its path relative to the cache root.
  fn cache(&self) -> Vec<(PathBuf, Vec<u8>)> {
    let root = self.path("builds/cache/decodes");

    walk(&root, &root)
  }

  fn assert_workspace_bytes(&self, vpath: &str, expected: &[u8]) {
    assert_eq!(fs::read(self.workspace_path(vpath)).unwrap(), expected);
  }
}

/// Every file under `dir`, keyed by its path relative to `root`.
fn walk(dir: &Path, root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
  let mut files: Vec<(PathBuf, Vec<u8>)> = fs::read_dir(dir)
    .unwrap()
    .flat_map(|entry| {
      let path = entry.unwrap().path();

      if path.is_dir() {
        walk(&path, root)
      } else {
        vec![(
          path.strip_prefix(root).unwrap().to_path_buf(),
          fs::read(&path).unwrap(),
        )]
      }
    })
    .collect();

  files.sort();
  files
}

/// The lines `ludared override list` printed.
fn lines(output: &str) -> Vec<&str> {
  output
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .collect()
}

#[test]
fn lists_nothing_before_anything_is_overridden() {
  let project = Project::unpacked();

  // An empty list is a valid answer, and never a failure.
  assert_eq!(project.ok(&["override", "list"]), "");
  assert_eq!(project.ok(&["override", "tree"]), "");
}

#[test]
fn lists_the_artifacts_declared_as_overrides() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  project.override_path(TITLE);

  assert_eq!(lines(&project.ok(&["override", "list"])), [TITLE, TAIL]);
}

#[test]
fn renders_the_overrides_as_a_tree() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  project.override_path(TITLE);

  assert_eq!(
    project.ok(&["override", "tree"]),
    "\
game.sfc
├── head.bin
│   └── TITLE.txt
└── tail.bin
"
  );
}

#[test]
fn renders_only_the_requested_branch_of_the_override_tree() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  project.override_path(TITLE);

  // An implicit parent is accepted even though it is not an artifact itself.
  assert_eq!(
    project.ok(&[
      "override",
      "tree",
      HEAD
    ]),
    "\
game.sfc/head.bin
└── TITLE.txt
"
  );
}

#[test]
fn filters_the_listing_by_a_prefix_and_its_descendants() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  project.override_path(TITLE);

  assert_eq!(
    lines(&project.ok(&[
      "override",
      "list",
      HEAD
    ])),
    [TITLE]
  );
  // A prefix that is not itself overridden still matches what is below it.
  assert_eq!(
    lines(&project.ok(&[
      "override",
      "list",
      SOURCE
    ])),
    [TITLE, TAIL]
  );
  // A path is included in its own listing.
  assert_eq!(
    lines(&project.ok(&[
      "override",
      "list",
      TAIL
    ])),
    [TAIL]
  );
  // A prefix that is neither an override nor a parent of one matches nothing.
  assert_eq!(
    project.ok(&[
      "override",
      "list",
      "game.sfc/head.bin/TITLE"
    ]),
    ""
  );
  // Neither does a branch holding no override.
  assert_eq!(
    project.ok(&[
      "override",
      "list",
      "elsewhere.sfc"
    ]),
    ""
  );
}

#[test]
fn lists_overrides_whose_workspace_files_are_gone() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  fs::remove_file(project.workspace_path(TAIL)).unwrap();

  // The manifest is the source of truth, not the filesystem.
  assert_eq!(lines(&project.ok(&["override", "list"])), [TAIL]);
}

#[test]
fn adds_an_override_into_the_workspace() {
  let project = Project::unpacked();

  project.override_path(TITLE);

  // The nested directories the virtual path implies are created along the way.
  project.assert_workspace_bytes(TITLE, b"HELL");
  assert!(project.declares_override("TITLE", "TITLE.txt"));
}

#[test]
fn records_an_override_on_the_decode_node_that_produced_it() {
  let project = Project::unpacked();

  project.override_path(TITLE);
  project.override_path(TAIL);

  // One node per overridden artifact, and nothing added to the node that
  // produced their parent.
  assert_eq!(project.override_fields(), 2);
  assert!(project.declares_override("TITLE", "TITLE.txt"));
  assert!(project.declares_override("tail", "tail.bin"));
  assert!(!project.declares_override("head", "head.bin"));
}

#[test]
fn leaves_the_manifest_free_of_override_fields_it_has_no_use_for() {
  let project = Project::unpacked();

  // Declaring nothing adds no field anywhere.
  assert_eq!(project.override_fields(), 0);
  assert!(project.run(&["override", "list"]).status.success());

  // Declaring one only touches the node that produced it.
  project.override_path(TAIL);
  assert_eq!(project.override_fields(), 1);

  // Lifting it again leaves the manifest as it was found.
  project.ok(&[
    "override",
    "remove",
    TAIL,
  ]);
  assert_eq!(project.override_fields(), 0);
}

#[test]
fn adds_an_override_under_the_fixed_workspace_path() {
  let project = Project::unpacked();

  project.override_path(TITLE);

  assert!(project.workspace_path(TITLE).is_file());
  assert_eq!(
    project.workspace_path(TITLE),
    project.path("workspace").join(HEAD).join("TITLE.txt")
  );
}

#[test]
fn refuses_a_path_that_is_no_decoded_artifact() {
  let project = Project::unpacked();

  for vpath in [
    // A source on its own.
    SOURCE,
    // An implicit parent, which is not an artifact of its own.
    "game.sfc/head.bin/TITLE",
    // Nothing was ever decoded under that name.
    "game.sfc/missing.bin",
    // Another source entirely.
    "elsewhere.sfc/tail.bin",
  ] {
    let stderr = project.fails(&[
      "override",
      "add",
      vpath,
    ]);

    assert!(
      stderr.contains("Artifact not found"),
      "`override add {vpath}` reported: {stderr}"
    );
    assert!(!project.workspace_path(vpath).exists());
  }
}

#[test]
fn refuses_an_artifact_with_decodes_of_its_own() {
  let project = Project::unpacked();

  let stderr = project.fails(&[
    "override",
    "add",
    HEAD,
  ]);

  assert!(
    stderr.contains("artifact is not terminal"),
    "unexpected error: {stderr}"
  );
  assert!(!project.workspace_path(HEAD).exists());
}

#[test]
fn refuses_an_artifact_that_is_already_overridden() {
  let project = Project::unpacked();

  project.override_path(TAIL);

  let stderr = project.fails(&[
    "override",
    "add",
    TAIL,
  ]);

  assert!(
    stderr.contains("already overridden"),
    "unexpected error: {stderr}"
  );
  assert!(stderr.contains("override refresh"), "{stderr}");
}

#[test]
fn refuses_an_already_overridden_artifact_even_with_force() {
  let project = Project::unpacked();

  project.override_path(TAIL);

  // Being overridden is about the marker, not about the file, so `force` cannot
  // take it over.
  let stderr = project.fails(&[
    "override",
    "add",
    TAIL,
    "--force",
  ]);

  assert!(
    stderr.contains("already overridden"),
    "unexpected error: {stderr}"
  );
  assert!(stderr.contains("override refresh"), "{stderr}");
}

#[test]
fn refuses_a_workspace_file_that_is_already_there() {
  let project = Project::unpacked();

  project.write_workspace(TAIL, b"HAND WRITTEN");

  let stderr = project.fails(&[
    "override",
    "add",
    TAIL,
  ]);

  assert!(
    stderr.contains("already exists"),
    "unexpected error: {stderr}"
  );
  assert!(stderr.contains("--force"), "{stderr}");
  // The operator's file is left alone.
  project.assert_workspace_bytes(TAIL, b"HAND WRITTEN");
  assert_eq!(project.ok(&["override", "list"]), "");
}

#[test]
fn replaces_a_workspace_file_with_force() {
  let project = Project::unpacked();

  project.write_workspace(TAIL, b"HAND WRITTEN");
  project.ok(&[
    "override",
    "add",
    TAIL,
    "--force",
  ]);

  project.assert_workspace_bytes(TAIL, b"ORLD");
  assert!(project.declares_override("tail", "tail.bin"));
}

#[test]
fn refuses_a_workspace_file_a_previous_remove_kept() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  project.ok(&[
    "override",
    "remove",
    TAIL,
  ]);

  // `remove` without `--clean` keeps the file, so a later `add` must not take it
  // over without being told to.
  let stderr = project.fails(&[
    "override",
    "add",
    TAIL,
  ]);

  assert!(
    stderr.contains("already exists"),
    "unexpected error: {stderr}"
  );
}

#[test]
fn refreshes_an_override_from_its_canonical_bytes() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  fs::write(project.workspace_path(TAIL), b"LOCAL EDIT").unwrap();

  project.ok(&[
    "override",
    "refresh",
    TAIL,
  ]);

  project.assert_workspace_bytes(TAIL, b"ORLD");
  assert!(project.declares_override("tail", "tail.bin"));
}

#[test]
fn refreshes_an_override_whose_file_was_deleted() {
  let project = Project::unpacked();

  project.override_path(TITLE);
  fs::remove_dir_all(project.path("workspace")).unwrap();

  project.ok(&[
    "override",
    "refresh",
    TITLE,
  ]);

  project.assert_workspace_bytes(TITLE, b"HELL");
}

#[test]
fn refuses_to_refresh_an_artifact_that_is_not_overridden() {
  let project = Project::unpacked();

  let stderr = project.fails(&[
    "override",
    "refresh",
    TAIL,
  ]);

  assert!(
    stderr.contains("not overridden"),
    "unexpected error: {stderr}"
  );
  assert!(!project.workspace_path(TAIL).exists());
}

#[test]
fn documents_that_refresh_discards_local_changes() {
  let project = Project::unpacked();

  let help = project.ok(&[
    "override",
    "refresh",
    "--help",
  ]);

  assert!(help.contains("discard"), "{help}");
}

#[test]
fn removes_an_override_and_keeps_the_workspace_file() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  fs::write(project.workspace_path(TAIL), b"MINE NOW").unwrap();

  project.ok(&[
    "override",
    "remove",
    TAIL,
  ]);

  assert_eq!(project.ok(&["override", "list"]), "");
  project.assert_workspace_bytes(TAIL, b"MINE NOW");
}

#[test]
fn cleans_up_only_the_targeted_file() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  project.override_path(TITLE);

  project.ok(&[
    "override",
    "remove",
    TAIL,
    "--clean",
  ]);

  assert!(!project.workspace_path(TAIL).exists());
  // The sibling override survives, and so does the directory the deleted file
  // used to sit in.
  project.assert_workspace_bytes(TITLE, b"HELL");
  assert!(project.workspace_path(TAIL).parent().unwrap().is_dir());
}

#[test]
fn cleans_up_a_file_that_is_already_gone() {
  let project = Project::unpacked();

  project.override_path(TAIL);
  fs::remove_file(project.workspace_path(TAIL)).unwrap();

  // Nothing left to delete is a cleanup that succeeded.
  project.ok(&[
    "override",
    "remove",
    TAIL,
    "--clean",
  ]);

  assert_eq!(project.ok(&["override", "list"]), "");
}

#[test]
fn keeps_the_override_declared_when_the_cleanup_fails() {
  let project = Project::unpacked();

  project.override_path(TAIL);

  // A directory where the workspace file belongs cannot be deleted.
  let workspace = project.workspace_path(TAIL);
  fs::remove_file(&workspace).unwrap();
  fs::create_dir(&workspace).unwrap();

  project.fails(&[
    "override",
    "remove",
    TAIL,
    "--clean",
  ]);

  // The file is cleaned before the declaration is dropped, so the failure leaves
  // the override active rather than losing track of a file that is still there.
  assert_eq!(lines(&project.ok(&["override", "list"])), [TAIL]);
}

#[test]
fn refuses_to_clean_a_file_that_is_not_overridden() {
  let project = Project::unpacked();

  project.write_workspace(TAIL, b"NOT AN OVERRIDE");

  let stderr = project.fails(&[
    "override",
    "remove",
    TAIL,
    "--clean",
  ]);

  assert!(
    stderr.contains("not overridden"),
    "unexpected error: {stderr}"
  );
  // An arbitrary file of the workspace is out of reach of `--clean`.
  project.assert_workspace_bytes(TAIL, b"NOT AN OVERRIDE");
}

#[test]
fn refuses_to_remove_an_artifact_that_is_not_overridden() {
  let project = Project::unpacked();

  let stderr = project.fails(&[
    "override",
    "remove",
    TAIL,
  ]);

  assert!(
    stderr.contains("not overridden"),
    "unexpected error: {stderr}"
  );
}

#[test]
fn has_no_implicit_override_of_a_path() {
  let project = Project::unpacked();

  // `ludared override <PATH>` is not a command: the subcommand is required, and
  // a virtual path in that position is not one.
  let stderr = {
    let output = project.run(&["override", TAIL]);

    assert!(!output.status.success());
    String::from_utf8_lossy(&output.stderr).into_owned()
  };

  assert!(stderr.contains("unrecognized subcommand"), "{stderr}");
  assert!(stderr.contains("<COMMAND>"), "{stderr}");

  // The five subcommands, and nothing that would take a bare path.
  let help = project.ok(&["override", "--help"]);

  for subcommand in [
    "list",
    "tree",
    "add",
    "refresh",
    "remove",
  ] {
    assert!(
      help.contains(subcommand),
      "`override --help` lacks {subcommand}: {help}"
    );
  }
}

#[test]
fn refuses_to_decode_into_an_overridden_artifact() {
  let project = Project::unpacked();

  project.override_path(TAIL);

  let args = r#"{"target":"inner.bin","offset":0,"length":2}"#;
  let stderr = project.fails(&[
    "decode",
    "add",
    TAIL,
    EXTRACT,
    args,
  ]);

  assert!(
    stderr.contains("artifact is overridden"),
    "unexpected error: {stderr}"
  );
  // The refusal happens before anything is decoded or cached.
  assert_eq!(lines(&project.ok(&["override", "list"])), [TAIL]);
}

#[test]
fn leaves_the_decode_cache_untouched() {
  let project = Project::unpacked();
  let before = project.cache();

  project.override_path(TAIL);
  project.override_path(TITLE);
  fs::write(project.workspace_path(TAIL), b"LOCAL EDIT").unwrap();
  project.ok(&[
    "override",
    "refresh",
    TAIL,
  ]);
  project.ok(&[
    "override",
    "remove",
    TAIL,
    "--clean",
  ]);
  project.ok(&["override", "list"]);
  project.ok(&["override", "tree"]);

  assert_eq!(project.cache(), before);
}

/// Drives the dynamic completion protocol the way a shell's completion function
/// does, and returns the candidates offered for `partial`.
///
/// `COMP_CWORD` is the index of the word being completed, which is the virtual
/// path here. The separator is returned as a control character, so the output is
/// split back apart on the one that was asked for.
fn complete(project: &Project, command: &[&str], partial: &str) -> Vec<String> {
  const IFS: &str = "\u{1b}";

  let output = Command::new(env!("CARGO_BIN_EXE_ludared"))
    .current_dir(project.dir.path())
    .env("COMPLETE", "bash")
    .env("_CLAP_IFS", IFS)
    .env("_CLAP_COMPLETE_INDEX", "3")
    .env("_CLAP_COMPLETE_COMP_TYPE", "9")
    .env("_CLAP_COMPLETE_SPACE", "true")
    .arg("--")
    .arg("ludared")
    .args(command)
    .arg(partial)
    .output()
    .unwrap();

  assert!(output.status.success());

  String::from_utf8(output.stdout)
    .unwrap()
    .split(IFS)
    // clap offers its own flags alongside; they are not what these commands are
    // completing.
    .filter(|candidate| candidate.contains('.'))
    .map(str::to_owned)
    .collect()
}

#[test]
fn completes_the_artifacts_an_override_may_be_declared_on() {
  let project = Project::unpacked();

  // `head.bin` decodes `TITLE.txt` and is not a leaf, so only the two leaves are
  // ever offered.
  assert_eq!(complete(&project, &["override", "add"], ""), [TITLE, TAIL]);
  assert_eq!(
    complete(&project, &["override", "add"], "game.sfc/head"),
    [TITLE]
  );
  assert_eq!(
    complete(&project, &["override", "add"], "game.sfc/t"),
    [TAIL]
  );
}

#[test]
fn completes_nothing_for_an_artifact_already_overridden() {
  let project = Project::unpacked();

  project.override_path(TITLE);

  // Declared already, so `add` has nothing to offer for it.
  assert_eq!(complete(&project, &["override", "add"], ""), [TAIL]);
  assert_eq!(
    complete(&project, &["override", "add"], HEAD),
    Vec::<String>::new()
  );
}

#[test]
fn completes_only_the_active_overrides_for_refresh_and_remove() {
  let project = Project::unpacked();

  assert_eq!(
    complete(
      &project,
      &[
        "override",
        "refresh"
      ],
      ""
    ),
    Vec::<String>::new()
  );
  assert_eq!(
    complete(&project, &["override", "remove"], ""),
    Vec::<String>::new()
  );

  project.override_path(TITLE);

  assert_eq!(
    complete(
      &project,
      &[
        "override",
        "refresh"
      ],
      ""
    ),
    [TITLE]
  );
  assert_eq!(complete(&project, &["override", "remove"], ""), [TITLE]);
  // Both act on a single artifact, so the parents standing for a branch of a
  // listing are never candidates of their own. Typed text still narrows the one
  // artifact there is, since completion matches text.
  assert_eq!(
    complete(&project, &["override", "remove"], "game.sfc/head.bin/TI"),
    [TITLE]
  );
  assert_eq!(
    complete(
      &project,
      &[
        "override",
        "refresh"
      ],
      "game.sfc/tail"
    ),
    Vec::<String>::new()
  );
}

#[test]
fn completes_the_overrides_and_their_implicit_parents_for_list_and_tree() {
  let project = Project::unpacked();

  assert_eq!(
    complete(&project, &["override", "list"], ""),
    Vec::<String>::new()
  );

  project.override_path(TITLE);

  let expected = [SOURCE, HEAD, TITLE];

  assert_eq!(complete(&project, &["override", "list"], ""), expected);
  assert_eq!(complete(&project, &["override", "tree"], ""), expected);
  assert_eq!(
    complete(&project, &["override", "list"], HEAD),
    [HEAD, TITLE]
  );
}

#[test]
fn completes_the_same_candidates_however_often_it_is_asked() {
  let project = Project::unpacked();

  project.override_path(TITLE);

  for _ in 0..5 {
    assert_eq!(
      complete(&project, &["override", "list"], ""),
      complete(&project, &["override", "list"], "")
    );
    assert_eq!(
      complete(&project, &["override", "add"], ""),
      complete(&project, &["override", "add"], "")
    );
  }
}

#[test]
fn completes_nothing_for_a_partial_path_matching_nothing() {
  let project = Project::unpacked();

  project.override_path(TITLE);

  for (subcommand, partial) in [
    ("add", "elsewhere.sfc"),
    ("refresh", "elsewhere.sfc"),
    ("remove", "elsewhere.sfc"),
    ("list", "elsewhere.sfc"),
    ("tree", "elsewhere.sfc"),
  ] {
    assert_eq!(
      complete(
        &project,
        &[
          "override",
          subcommand
        ],
        partial
      ),
      Vec::<String>::new(),
      "`override {subcommand} {partial}` should complete nothing"
    );
  }
}

#[test]
fn lists_the_sources_and_outputs_the_manifest_declares() {
  // Nothing has been unpacked: what the manifest declares is an artifact whether
  // or not a decode has ever been replayed.
  let project = Project::unpacked_source_tree();

  assert_eq!(
    lines(&project.ok(&["artifacts", "list"])),
    [
      SOURCE,
      HEAD,
      TITLE,
      "game.sfc/tail.bin",
    ]
  );
}

#[test]
fn renders_the_declared_artifacts_as_a_tree() {
  let project = Project::unpacked_source_tree();

  // The parents are drawn as the branches leading to the outputs, without ever
  // being artifacts themselves.
  assert_eq!(
    project.ok(&["artifacts", "tree"]),
    "\
game.sfc
├── head.bin
│   └── TITLE.txt
└── tail.bin
"
  );
}

#[test]
fn filters_the_artifacts_by_a_subtree_of_whole_components() {
  let project = Project::unpacked_source_tree();

  // A path selects itself along with what it holds.
  assert_eq!(
    lines(&project.ok(&[
      "artifacts",
      "list",
      SOURCE
    ])),
    [
      SOURCE,
      HEAD,
      TITLE,
      "game.sfc/tail.bin",
    ]
  );
  // A path selects itself along with what it holds.
  assert_eq!(
    lines(&project.ok(&[
      "artifacts",
      "list",
      HEAD
    ])),
    [HEAD, TITLE]
  );
  // A prefix of a component is not that component.
  assert_eq!(
    project.ok(&[
      "artifacts",
      "list",
      "game.sfc/head.b"
    ]),
    ""
  );
}

#[test]
fn renders_only_the_requested_branch_of_the_tree() {
  let project = Project::unpacked_source_tree();

  assert_eq!(
    project.ok(&[
      "artifacts",
      "tree",
      HEAD
    ]),
    "\
game.sfc/head.bin
└── TITLE.txt
"
  );
}

#[test]
fn renders_nothing_for_a_root_declaring_no_artifact() {
  let project = Project::unpacked_source_tree();

  assert_eq!(
    project.ok(&[
      "artifacts",
      "tree",
      "elsewhere.sfc"
    ]),
    ""
  );
  assert_eq!(
    project.ok(&[
      "artifacts",
      "list",
      "elsewhere.sfc"
    ]),
    ""
  );
}

#[test]
fn lists_the_same_artifacts_whether_or_not_the_cache_is_populated() {
  let unpacked = Project::unpacked();
  let bare = Project::unpacked_source_tree();

  assert_eq!(
    project_ok(&unpacked, &["artifacts", "list"]),
    project_ok(&bare, &["artifacts", "list"])
  );
  assert_eq!(
    project_ok(&unpacked, &["artifacts", "tree"]),
    project_ok(&bare, &["artifacts", "tree"])
  );
}

#[test]
fn no_longer_offers_the_cache_a_listing_or_a_tree() {
  let project = Project::unpacked();

  for subcommand in ["list", "tree"] {
    let stderr = {
      let output = project.run(&["cache", subcommand]);

      assert!(
        !output.status.success(),
        "`cache {subcommand}` still exists"
      );
      String::from_utf8_lossy(&output.stderr).into_owned()
    };

    assert!(
      stderr.contains("unrecognized subcommand"),
      "`cache {subcommand}` reported: {stderr}"
    );
  }

  // The two that do read the physical cache are untouched.
  let help = project.ok(&["cache", "--help"]);

  assert!(help.contains("cat"), "{help}");
  assert!(help.contains("path"), "{help}");
  assert!(!help.contains("list"), "{help}");
  assert!(!help.contains("tree"), "{help}");
}

#[test]
fn still_reads_the_bytes_and_the_path_of_a_cached_artifact() {
  let project = Project::unpacked();

  assert_eq!(
    project.ok(&[
      "cache",
      "cat",
      TITLE
    ]),
    "HELL"
  );

  let blob = project.ok(&[
    "cache",
    "path",
    TITLE,
  ]);

  assert!(blob.trim().ends_with(&sha256_of(b"HELL")), "{blob}");

  // An artifact the manifest declares but nothing has cached is still an
  // artifact, and still nothing to read from the cache.
  let project = Project::unpacked_source_tree();

  assert_eq!(
    lines(&project.ok(&["artifacts", "list"])),
    [
      SOURCE,
      HEAD,
      TITLE,
      "game.sfc/tail.bin"
    ]
  );
  assert!(
    !project
      .fails(&[
        "cache",
        "cat",
        TITLE
      ])
      .is_empty()
  );
  assert!(
    !project
      .fails(&[
        "cache",
        "path",
        TITLE
      ])
      .is_empty()
  );
}

#[test]
fn completes_artifacts_from_the_manifest_rather_than_the_cache() {
  let bare = Project::unpacked_source_tree();

  // With an empty cache, the manifest is all there is to complete from.
  assert_eq!(
    complete(&bare, &["artifacts", "list"], ""),
    [
      SOURCE,
      HEAD,
      TITLE,
      "game.sfc/tail.bin"
    ]
  );
  assert_eq!(
    complete(&bare, &["artifacts", "tree"], ""),
    complete(&bare, &["artifacts", "list"], "")
  );
  assert_eq!(
    complete(&bare, &["artifacts", "list"], "game.sfc/head"),
    [HEAD, TITLE]
  );
  // A prefix of a component is not that component.
  // Completion matches typed text rather than whole components, so a half-typed
  // component still completes the paths carrying it.
  assert_eq!(
    complete(&bare, &["artifacts", "list"], "game.sfc/head.b"),
    [HEAD, TITLE]
  );
}

/// Runs a command and returns what it printed, asserting that it succeeded.
fn project_ok(project: &Project, args: &[&str]) -> String {
  project.ok(args)
}

/// The content-addressed name a blob of `bytes` is stored under.
fn sha256_of(bytes: &[u8]) -> String {
  use sha2::Digest as _;

  sha2::Sha256::digest(bytes)
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect()
}

/// Decodes `target` out of the source, as a second decode operation on it.
fn decode(project: &Project, target: &str) -> std::process::Output {
  let args = format!(r#"{{"target":"{target}","offset":0,"length":4}}"#);

  project.run(&[
    "decode",
    "add",
    SOURCE,
    "std/generic/extract_bytes",
    &args,
  ])
}

#[test]
fn decodes_artifacts_sharing_an_implicit_directory() {
  let project = Project::unpacked_source_tree();

  for target in [
    "data/gfx/head.bin",
    "data/gfx/tail.bin",
    "data/sound.bin",
  ] {
    assert!(
      decode(&project, target).status.success(),
      "'{target}' should be a valid output"
    );
  }

  // The fixture's own decode tree is still there alongside the new artifacts.
  assert_eq!(
    lines(&project.ok(&[
      "artifacts",
      "list",
      "game.sfc/data"
    ])),
    [
      "game.sfc/data/gfx/head.bin",
      "game.sfc/data/gfx/tail.bin",
      "game.sfc/data/sound.bin",
    ]
  );
}

#[test]
fn refuses_to_decode_an_artifact_beneath_a_sibling_decodes() {
  let project = Project::unpacked_source_tree();

  assert!(decode(&project, "data").status.success());

  let stderr = {
    let output = decode(&project, "data/gfx/head.bin");

    assert!(!output.status.success());
    String::from_utf8_lossy(&output.stderr).into_owned()
  };

  // The conflict is with what the sibling decode already declared, not with the
  // codec that produced the output.
  assert!(
    stderr.contains("Manifest Error: Overlapping artifacts"),
    "{stderr}"
  );
  // Nothing was recorded for the refused decode.
  assert_eq!(
    lines(&project.ok(&[
      "artifacts",
      "list",
      "game.sfc/data"
    ])),
    ["game.sfc/data"]
  );
}

#[test]
fn refuses_to_decode_the_same_output_twice() {
  let project = Project::unpacked_source_tree();

  assert!(decode(&project, "dup.bin").status.success());

  // The decode is named apart from the output, so what is refused here is the
  // output it would record rather than the name of the operation.
  let stderr = {
    let output = project.run(&[
      "decode",
      "add",
      SOURCE,
      "std/generic/extract_bytes",
      r#"{"target":"dup.bin","offset":0,"length":2}"#,
      "--name",
      "dup-again",
    ]);

    assert!(!output.status.success());
    String::from_utf8_lossy(&output.stderr).into_owned()
  };

  // An identical name is a duplicate rather than an overlap.
  assert!(stderr.contains("Duplicate output name"), "{stderr}");
}

#[test]
fn decodes_artifacts_nested_beneath_another_decode_output() {
  let project = Project::unpacked_source_tree();

  // A decode may produce an artifact, which a later decode on that artifact may in
  // turn produce beneath: the two live at different levels of the tree.
  assert!(decode(&project, "bank.bin").status.success());

  let output = project.run(&[
    "decode",
    "add",
    "game.sfc/bank.bin",
    "std/generic/extract_bytes",
    r#"{"target":"inner.bin","offset":0,"length":2}"#,
  ]);

  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );
  assert_eq!(
    lines(&project.ok(&[
      "artifacts",
      "list",
      "game.sfc/bank.bin"
    ])),
    [
      "game.sfc/bank.bin",
      "game.sfc/bank.bin/inner.bin"
    ]
  );
}

#[test]
fn refuses_to_clean_the_workspace_file_of_a_path_that_is_no_artifact() {
  let project = Project::unpacked();

  // Nothing was ever decoded here, so there is nothing to clean. The declaration
  // is what the check turns on, and this path holds none.
  let stderr = project.fails(&[
    "override",
    "remove",
    "game.sfc/missing.bin",
    "--clean",
  ]);

  assert!(stderr.contains("not overridden"), "{stderr}");
  assert!(!project.workspace_path("game.sfc/missing.bin").exists());
}

#[test]
fn cleans_up_a_workspace_file_nobody_overrode() {
  let project = Project::unpacked();

  // A file sitting where an override would go must not be reachable through
  // `--clean`, however plausible it looks.
  project.write_workspace(TAIL, b"NOT AN OVERRIDE");

  let stderr = project.fails(&[
    "override",
    "remove",
    TAIL,
    "--clean",
  ]);

  assert!(stderr.contains("not overridden"), "{stderr}");
  assert_eq!(
    std::fs::read(project.workspace_path(TAIL)).unwrap(),
    b"NOT AN OVERRIDE"
  );
}

#[test]
fn overrides_an_artifact_that_was_just_decoded() {
  let project = Project::unpacked_source_tree();

  // Decoding an artifact leaves an empty decode bucket attached to it, which must
  // not stop that artifact from being a leaf an override can be declared on.
  let args = r#"{"target":"fresh.bin","offset":0,"length":4}"#;
  let output = project.run(&[
    "decode",
    "add",
    SOURCE,
    EXTRACT,
    args,
  ]);

  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );

  project.override_path("game.sfc/fresh.bin");

  assert_eq!(
    project.ok(&[
      "cache",
      "cat",
      "game.sfc/fresh.bin"
    ]),
    "HELL"
  );
  assert_eq!(
    lines(&project.ok(&["override", "list"])),
    ["game.sfc/fresh.bin"]
  );
}
