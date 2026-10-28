//! End-to-end coverage of `ludared decode list`.

use std::fs;
use std::process::Command;

use tempfile::TempDir;

const SOURCE: &str = "game.sfc";
const SOURCE_BYTES: &[u8] = b"HELLOWORLD";
const EXTRACT: &str = "std/generic/extract";

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
name = "decode-list-e2e"
manifest = "game.ludared"

[paths]
sources = "sources"
workspace = "workspace"
builds = "builds"
cache = "builds/cache"
"#,
    )
    .unwrap();

    fs::create_dir_all(dir.path().join("sources")).unwrap();
    fs::write(dir.path().join("sources").join(SOURCE), SOURCE_BYTES).unwrap();

    let manifest = format!(
      r#"{{
  "sources": {{
    "{SOURCE}": {{ "sha256": "0b21b7db59cd154904fac6336fa7d2be1bab38d632794f281549584068cdcb74", "size": null, "label": null }}
  }},
  "decodes": {{
    "{SOURCE}": [
      {{
        "name": "head",
        "codec": {{ "id": "{EXTRACT}", "version": 1, "args": null }},
        "outputs": [ "head.bin" ],
        "decodes": {{
          "head.bin": [
            {{
              "name": "TITLE",
              "codec": {{ "id": "{EXTRACT}", "version": 1, "args": null }},
              "outputs": [ "TITLE.txt" ],
              "decodes": {{}}
            }}
          ]
        }}
      }},
      {{
        "name": "very_long_decode_name",
        "codec": {{ "id": "{EXTRACT}", "version": 1, "args": null }},
        "outputs": [ "tail.bin" ],
        "decodes": {{}}
      }}
    ]
  }}
}}"#
    );
    fs::write(dir.path().join("game.ludared"), manifest).unwrap();

    Self { dir }
  }

  fn run(&self, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_ludared"))
      .current_dir(self.dir.path())
      .args(args)
      .output()
      .unwrap();

    (
      output.status.code().unwrap_or(-1),
      String::from_utf8(output.stdout).unwrap(),
      String::from_utf8(output.stderr).unwrap(),
    )
  }
}

#[test]
fn lists_only_direct_decodes_of_an_artifact() {
  let project = Project::new();

  let (code, stdout, stderr) = project.run(&[
    "decode",
    "list",
    SOURCE,
  ]);
  assert_eq!(code, 0, "stderr: {stderr}");
  assert_eq!(
    stdout,
    "head                  std/generic/extract\nvery_long_decode_name std/generic/extract\n"
  );
}

#[test]
fn lists_nested_decodes_without_descendants() {
  let project = Project::new();

  let (code, stdout, stderr) = project.run(&[
    "decode",
    "list",
    "game.sfc/head.bin",
  ]);
  assert_eq!(code, 0, "stderr: {stderr}");
  assert_eq!(stdout, "TITLE std/generic/extract\n");
}

#[test]
fn lists_nothing_for_an_artifact_without_decodes() {
  let project = Project::new();

  let (code, stdout, stderr) = project.run(&[
    "decode",
    "list",
    "game.sfc/tail.bin",
  ]);
  assert_eq!(code, 0, "stderr: {stderr}");
  assert_eq!(stdout, "");
}

#[test]
fn fails_for_an_unknown_artifact() {
  let project = Project::new();

  let (code, _stdout, stderr) = project.run(&[
    "decode",
    "list",
    "nope.sfc",
  ]);
  assert_ne!(code, 0);
  assert!(!stderr.is_empty());
}
