//! End-to-end coverage of `ludared build`.
//!
//! These tests drive the real binary against a throwaway project, so they cover
//! the parts a unit test cannot: argument wiring, the exit code a business case
//! produces, and the wording an operator is pointed at by a failure. Each test
//! owns its project directory and runs the binary with that directory as its
//! working directory, which is what the CLI resolves a project from.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::TempDir;

const SOURCE: &str = "game.sfc";
const OTHER_SOURCE: &str = "other.sfc";
const SOURCE_BYTES: &[u8] = b"HELLOWORLD";
const EXTRACT: &str = "std/generic/extract";
const SPLIT: &str = "std/generic/split";
const SPLIT_SOURCE_BYTES: &[u8] = b"HEADBODYTAIL";

struct Project {
  dir: TempDir,
}

impl Project {
  fn new() -> Self {
    let dir = TempDir::new().unwrap();

    fs::write(
      dir.path().join("ludared.toml"),
      r#"
[project]
name = "build-e2e"
manifest = "game.ludared"

[paths]
sources = "sources"
workspace = "workspace"
scripts = "scripts"
builds = "builds"
cache = "builds/cache"
"#,
    )
    .unwrap();
    fs::write(dir.path().join("game.ludared"), r#"{ "sources": {} }"#).unwrap();

    Self { dir }
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

  fn write_source(&self, name: &str, bytes: &[u8]) {
    fs::create_dir_all(self.path("sources")).unwrap();
    fs::write(self.path("sources").join(name), bytes).unwrap();
  }
}

/// A project holding one registered source, decoded and unpacked.
fn decoded_project() -> Project {
  let project = Project::new();

  project.write_source(SOURCE, SOURCE_BYTES);
  project.ok(&[
    "sources",
    "add",
    SOURCE,
  ]);
  project.ok(&[
    "decode",
    "add",
    SOURCE,
    EXTRACT,
    r#"{"name":"head.bin","offset":0,"length":5}"#,
  ]);
  project.ok(&["unpack"]);

  project
}

/// A project holding one registered source, decoded with split and unpacked.
fn split_decoded_project() -> Project {
  let project = Project::new();

  project.write_source(SOURCE, SPLIT_SOURCE_BYTES);
  project.ok(&[
    "sources",
    "add",
    SOURCE,
  ]);
  project.ok(&[
    "decode",
    "add",
    SOURCE,
    SPLIT,
    r#"{"offsets":[4,8]}"#,
  ]);
  project.ok(&["unpack"]);

  project
}

/// A project with a nested decode tree and three overrides:
///
/// HEADBODYTAIL
/// └── split [4,8]
///     ├── part_0.bin: HEAD
///     │   └── split [3]
///     │       ├── part_0.bin: HEA
///     │       └── part_1.bin: D → override: DER
///     ├── part_1.bin: BODY → override: LONGER
///     └── part_2.bin: TAIL
///         └── extract [1,3] name=extracted.bin
///             └── AI → override: OO
///
/// Expected rebuild: HEADERLONGERTOOL
fn nested_split_extract_project() -> Project {
  let project = Project::new();

  project.write_source(SOURCE, SPLIT_SOURCE_BYTES);
  project.ok(&[
    "sources",
    "add",
    SOURCE,
  ]);

  // Root split: HEADBODYTAIL → HEAD | BODY | TAIL
  project.ok(&[
    "decode",
    "add",
    SOURCE,
    SPLIT,
    r#"{"offsets":[4,8]}"#,
  ]);

  // Nested split on part_0.bin (HEAD): HEAD → HEA | D
  project.ok(&[
    "decode",
    "add",
    "game.sfc/part_0.bin",
    SPLIT,
    r#"{"offsets":[3]}"#,
  ]);

  // Nested extract on part_2.bin (TAIL): TAIL → extract "AI" at offset 1, length 3
  project.ok(&[
    "decode",
    "add",
    "game.sfc/part_2.bin",
    EXTRACT,
    r#"{"name":"extracted.bin","offset":1,"length":3}"#,
  ]);

  project.ok(&["unpack"]);

  // Declare and write the three overrides
  project.ok(&[
    "override",
    "add",
    "game.sfc/part_0.bin/part_1.bin",
  ]);
  fs::write(
    project
      .path("workspace")
      .join("game.sfc/part_0.bin/part_1.bin"),
    b"DER",
  )
  .unwrap();

  project.ok(&[
    "override",
    "add",
    "game.sfc/part_1.bin",
  ]);
  fs::write(
    project.path("workspace").join("game.sfc/part_1.bin"),
    b"LONGER",
  )
  .unwrap();

  project.ok(&[
    "override",
    "add",
    "game.sfc/part_2.bin/extracted.bin",
  ]);
  fs::write(
    project
      .path("workspace")
      .join("game.sfc/part_2.bin/extracted.bin"),
    b"OOL",
  )
  .unwrap();

  project
}

#[test]
fn builds_the_only_source_of_a_project() {
  let project = decoded_project();

  assert_eq!(project.ok(&["build"]), "✓ Built builds/game.sfc\n");
  assert_eq!(
    fs::read(project.path("builds").join(SOURCE)).unwrap(),
    SOURCE_BYTES
  );
}

#[test]
fn overwrites_a_previous_build_without_asking() {
  let project = decoded_project();

  project.ok(&["build"]);
  fs::write(project.path("builds").join(SOURCE), b"STALE").unwrap();

  project.ok(&["build"]);

  assert_eq!(
    fs::read(project.path("builds").join(SOURCE)).unwrap(),
    SOURCE_BYTES
  );
}

#[test]
fn writes_workspace_edits_back_into_the_image() {
  let project = decoded_project();

  project.ok(&[
    "override",
    "add",
    &format!("{SOURCE}/head.bin"),
  ]);
  fs::write(
    project.path("workspace").join(SOURCE).join("head.bin"),
    b"HEAD!",
  )
  .unwrap();

  project.ok(&["build"]);

  assert_eq!(
    fs::read(project.path("builds").join(SOURCE)).unwrap(),
    b"HEAD!WORLD"
  );
}

#[test]
fn builds_with_split_codec_without_overrides() {
  let project = split_decoded_project();

  assert_eq!(project.ok(&["build"]), "✓ Built builds/game.sfc\n");
  assert_eq!(
    fs::read(project.path("builds").join(SOURCE)).unwrap(),
    SPLIT_SOURCE_BYTES
  );
}

#[test]
fn writes_workspace_edits_back_into_the_image_with_split() {
  let project = split_decoded_project();

  project.ok(&[
    "override",
    "add",
    "game.sfc/part_1.bin",
  ]);
  fs::write(
    project
      .path("workspace")
      .join("game.sfc")
      .join("part_1.bin"),
    b"LONGER",
  )
  .unwrap();

  project.ok(&["build"]);

  assert_eq!(
    fs::read(project.path("builds").join(SOURCE)).unwrap(),
    b"HEADLONGERTAIL"
  );
}

#[test]
fn rebuilds_nested_decode_tree_with_multiple_overrides() {
  let project = nested_split_extract_project();

  project.ok(&["build"]);

  assert_eq!(
    fs::read(project.path("builds").join(SOURCE)).unwrap(),
    b"HEADERLONGERTOOL"
  );
}

#[test]
fn builds_every_source_of_a_project_by_default() {
  let project = Project::new();

  project.write_source(SOURCE, SOURCE_BYTES);
  project.write_source(OTHER_SOURCE, b"GOODBYE");
  project.ok(&[
    "sources",
    "add",
    SOURCE,
  ]);
  project.ok(&[
    "sources",
    "add",
    OTHER_SOURCE,
  ]);

  assert_eq!(
    project.ok(&["build"]),
    "✓ Built builds/game.sfc\n✓ Built builds/other.sfc\n"
  );
  assert_eq!(
    fs::read(project.path("builds").join(SOURCE)).unwrap(),
    SOURCE_BYTES
  );
  assert_eq!(
    fs::read(project.path("builds").join(OTHER_SOURCE)).unwrap(),
    b"GOODBYE"
  );
}

#[test]
fn builds_only_the_named_source() {
  let project = Project::new();

  project.write_source(SOURCE, SOURCE_BYTES);
  project.write_source(OTHER_SOURCE, b"GOODBYE");
  project.ok(&[
    "sources",
    "add",
    SOURCE,
  ]);
  project.ok(&[
    "sources",
    "add",
    OTHER_SOURCE,
  ]);

  assert_eq!(project.ok(&["build", SOURCE]), "✓ Built builds/game.sfc\n");
  assert_eq!(
    fs::read(project.path("builds").join(SOURCE)).unwrap(),
    SOURCE_BYTES
  );
  assert!(
    !project.path("builds").join(OTHER_SOURCE).exists(),
    "the other source should not have been built"
  );
}

#[test]
fn fails_when_the_project_has_no_sources() {
  let project = Project::new();

  let stderr = project.fails(&["build"]);

  assert!(
    stderr.contains("No source files defined in project"),
    "unexpected error: {stderr}"
  );
}

#[test]
fn fails_on_an_unknown_source() {
  let project = Project::new();
  project.write_source(SOURCE, SOURCE_BYTES);
  project.ok(&[
    "sources",
    "add",
    SOURCE,
  ]);

  let stderr = project.fails(&[
    "build",
    "missing.sfc",
  ]);

  assert!(
    stderr.contains("Source not found in manifest"),
    "unexpected error: {stderr}"
  );
}

#[test]
fn stops_at_the_first_failing_source() {
  let project = Project::new();

  project.write_source("a.sfc", b"AAA");
  project.write_source("b.sfc", b"BBB");
  project.ok(&[
    "sources",
    "add",
    "a.sfc",
  ]);
  project.ok(&[
    "sources",
    "add",
    "b.sfc",
  ]);

  // a.sfc can no longer be read, so it fails before b.sfc is reached.
  fs::remove_file(project.path("sources").join("a.sfc")).unwrap();

  let stderr = project.fails(&["build"]);

  assert!(stderr.contains("a.sfc"), "unexpected error: {stderr}");
  assert!(
    !project.path("builds").join("b.sfc").exists(),
    "b.sfc should not have been built after a.sfc failed"
  );
}

#[test]
fn decode_add_rejects_source_bytes_mismatch_manifest() {
  let project = Project::new();

  project.write_source(SOURCE, SOURCE_BYTES);
  project.ok(&[
    "sources",
    "add",
    SOURCE,
  ]);

  // Corrupt the source after registering its SHA-256.
  project.write_source(SOURCE, b"HELLOWORL\0");

  let stderr = project.fails(&[
    "decode",
    "add",
    SOURCE,
    EXTRACT,
    r#"{"name":"head.bin","offset":0,"length":5}"#,
  ]);

  assert!(
    stderr.contains("Invalid SHA256 for source file"),
    "expected a SHA256 mismatch, got: {stderr}"
  );
}
