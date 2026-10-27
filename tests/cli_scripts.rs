//! End-to-end coverage of script discovery and execution through the real CLI.
//! Each test owns its project; script output proves execution without depending
//! on filesystem traversal order.

use std::fs;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;

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
name = "scripts-e2e"
manifest = "game.ludared"

[paths]
scripts = "lua"
workspace = "editable"
"#,
    )
    .unwrap();
    fs::write(dir.path().join("game.ludared"), r#"{ "sources": {} }"#).unwrap();
    Self { dir }
  }

  fn write_script(&self, relative: &str, source: &str) {
    let path = self.dir.path().join("lua").join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
  }

  fn run(&self, selection: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ludared"))
      .current_dir(self.dir.path())
      .args(["scripts", "test"])
      .args(selection)
      .output()
      .unwrap()
  }

  fn ok(&self, selection: &[&str]) -> String {
    let output = self.run(selection);
    assert!(
      output.status.success(),
      "`ludared scripts test {}` failed: {}",
      selection.join(" "),
      String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
  }
}

#[test]
fn discovers_and_runs_tests_at_any_depth_under_the_configured_scripts_root() {
  let project = Project::new();
  project.write_script("tests/root.lua", "print('root')");
  project.write_script("tests/nested/child.lua", "print('child')");
  project.write_script("namespace/tests/deeper/feature.lua", "print('feature')");

  let stdout = project.ok(&[]);
  let mut lines: Vec<_> = stdout.lines().collect();
  lines.sort_unstable();
  assert_eq!(
    lines,
    [
      "child",
      "feature",
      "root"
    ]
  );
}

#[test]
fn skips_shared_subtrees_and_non_lua_files() {
  let project = Project::new();
  project.write_script("tests/run.lua", "print('ran')");
  for path in [
    "shared/tests/hidden.lua",
    "tests/shared/fixture.lua",
    "tests/nested/shared/fixture.lua",
    "namespace/shared/tests/hidden.lua",
    "tests/notes.txt",
  ] {
    project.write_script(path, "error('this file must not execute')");
  }

  assert_eq!(project.ok(&[]).trim(), "ran");
}

#[test]
fn runs_only_the_selected_script_relative_to_the_scripts_root() {
  let project = Project::new();
  project.write_script("tests/selected.lua", "print('selected')");
  project.write_script("tests/other.lua", "error('unselected test executed')");

  assert_eq!(project.ok(&["tests/selected.lua"]).trim(), "selected");
}

#[test]
fn reports_a_missing_script_without_running_other_tests() {
  let project = Project::new();
  project.write_script("tests/other.lua", "print('must not run')");

  let output = project.run(&["tests/missing.lua"]);
  assert_eq!(output.status.code(), Some(1));
  assert!(output.stdout.is_empty());
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(
    stderr.contains("Script not found: tests/missing.lua"),
    "{stderr}"
  );
}

#[test]
fn applies_the_glob_to_select_which_test_scripts_run() {
  let project = Project::new();
  project.write_script("tests/selected_first.lua", "print('first')");
  project.write_script("tests/selected_second.lua", "print('second')");
  project.write_script("tests/other.lua", "error('unmatched test executed')");

  // Command passes the pattern literally, so selection must happen in Ludared.
  let stdout = project.ok(&["tests/selected_*.lua"]);
  let mut lines: Vec<_> = stdout.lines().collect();
  lines.sort_unstable();
  assert_eq!(lines, ["first", "second"]);
}

#[test]
fn reports_an_unmatched_glob_without_running_other_tests() {
  let project = Project::new();
  project.write_script("tests/other.lua", "print('must not run')");

  let output = project.run(&["tests/missing_*.lua"]);
  assert_eq!(output.status.code(), Some(1));
  assert!(output.stdout.is_empty());
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(
    stderr.contains("No glob matches for pattern 'tests/missing_*.lua'"),
    "{stderr}"
  );
}

#[test]
fn provides_project_paths_and_ludared_functions_to_tests() {
  let project = Project::new();
  fs::create_dir_all(project.dir.path().join("editable")).unwrap();
  fs::write(project.dir.path().join("editable/input.bin"), "abc").unwrap();
  project.write_script("tests/shared/fixture.lua", "return 'abc'");
  project.write_script(
    "tests/environment.lua",
    r#"
local function read(path)
  local file = assert(io.open(path, "rb"))
  local data = file:read("*a")
  file:close()
  return data
end

assert(read(LUDARED.PROJECT_ROOT .. "/game.ludared") == '{ "sources": {} }')
local data = read(LUDARED.WORKSPACE .. "/input.bin")
assert(data == dofile(LUDARED.SCRIPTS .. "/tests/shared/fixture.lua"))
assert(LUDARED.sha256(data) ==
  "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")

local codec = LUDARED.codec("std/generic/extract")
local artifacts = codec:decode(data, '{"name":"middle.bin","offset":1,"length":1}')
assert(#artifacts == 1)
assert(artifacts[1].name == "middle.bin")
assert(artifacts[1].data == "b")
print("environment available")
"#,
  );

  assert_eq!(project.ok(&[]).trim(), "environment available");
}

#[test]
fn reports_lua_failures_with_a_failing_exit_status_and_script_path() {
  for (source, message) in [
    ("assert(false, 'broken invariant')", "broken invariant"),
    ("error('runtime failure')", "runtime failure"),
    ("local =", "syntax error"),
  ] {
    let project = Project::new();
    project.write_script("tests/failing.lua", source);

    let output = project.run(&[]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(message), "{stderr}");
    assert!(stderr.contains("tests/failing.lua"), "{stderr}");
  }
}

#[test]
fn isolates_lua_state_between_test_scripts() {
  let project = Project::new();
  // Either file may execute first; both must see a fresh VM.
  for path in [
    "tests/first.lua",
    "tests/second.lua",
  ] {
    project.write_script(
      path,
      "assert(test_has_run == nil); test_has_run = true; print('isolated')",
    );
  }

  assert_eq!(
    project.ok(&[]).lines().collect::<Vec<_>>(),
    [
      "isolated",
      "isolated"
    ]
  );
}
