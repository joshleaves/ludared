//! End-to-end coverage of the Lua codec boundary.
//!
//! These tests drive the real binary against a throwaway project, so they cover
//! the parts a unit test cannot: which Lua files of a project are picked up as
//! codecs, the ids the registry hands the rest of the CLI for them, and what a
//! codec is handed for its arguments each time it is called. Each test owns its
//! project directory and runs the binary with that directory as its working
//! directory, which is what the CLI resolves a project from.
//!
//! Discovery is observed the way every consumer of the registry observes it: by
//! asking for a codec by id. `decode add` resolves its codec before it touches
//! the virtual path, so the id is answered on its own — an id the registry never
//! issued is reported as unavailable, while an id it did issue is handed to that
//! codec.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;

const SOURCE: &str = "game.sfc";
const SOURCE_BYTES: &[u8] = b"HELLOWORLD";
const EXTRACT: &str = "std/generic/extract";

/// A codec at `scripts/sakura/text.lua`, named by the path it lives at rather
/// than by anything the module itself declares.
const SAKURA_TEXT: &str = "scripts/sakura/text.lua";

/// The same codec with the extension left off, which is not how the registry
/// spells it.
const SAKURA_TEXT_WITHOUT_EXTENSION: &str = "scripts/sakura/text";

/// The smallest module a Lua codec is loaded from: the fields its constructor
/// reads, and the four functions every codec is required to carry.
///
/// It is a `Patch` codec pulling the middle byte of its input into an artifact of
/// its own. `AA BB CC` leaves `BB` behind, and putting a byte back there rebuilds
/// the input around it.
///
/// Everything either side of that byte is carried through rather than dropped, so
/// an input of any width round trips exactly: the parts around the artifact are
/// read back off the parent the codec is handed at encode time.
const LUA_CODEC: &str = r#"
return {
  name = "Middle byte",
  kind = "patch",
  description = "Extracts the middle byte of a three-byte input.",
  version = 1,
  can_handle = function(data) return "possible" end,
  decode_name = function(args) return "middle" end,
  decode = function(data, args)
    return { { name = "middle.bin", data = data:sub(2, 2) } }
  end,
  encode = function(original, artifacts, args)
    return original:sub(1, 1) .. artifacts[1].data .. original:sub(3)
  end,
}
"#;

/// A codec that reports the arguments it was handed by writing them into what it
/// rebuilds, so that what reached `encode` is readable in the build output.
///
/// The encoded parent is `tostring(args)`, a NUL, and the artifact, which makes
/// the three states an argument can be in distinguishable from the outside: a
/// recorded string, an empty string, and no argument at all (`nil`).
const LUA_ECHO_ARGS: &str = r#"
return {
  name = "Echo arguments",
  kind = "patch",
  description = "Writes the arguments it was handed into its output.",
  version = 1,
  can_handle = function(data) return "likely" end,
  decode_name = function(args) return "echo" end,
  decode = function(data, args)
    return { { name = "echo.bin", data = data } }
  end,
  encode = function(original, artifacts, args)
    return tostring(args) .. "\0" .. artifacts[1].data
  end,
}
"#;

struct Project {
  dir: TempDir,
}

impl Project {
  /// A directory resolving no project at all, for the commands that have to work
  /// outside one.
  fn outside_a_project() -> Self {
    Self {
      dir: TempDir::new().unwrap(),
    }
  }

  fn new() -> Self {
    let dir = TempDir::new().unwrap();

    fs::write(
      dir.path().join("ludared.toml"),
      r#"
[project]
name = "codecs-e2e"
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

  /// A project with one registered source, which is what decodes are run on.
  fn with_source() -> Self {
    Self::with_source_bytes(SOURCE_BYTES)
  }

  /// A project with one registered source holding `bytes`.
  fn with_source_bytes(bytes: &[u8]) -> Self {
    let project = Self::new();

    fs::create_dir_all(project.path("sources")).unwrap();
    fs::write(project.path("sources").join(SOURCE), bytes).unwrap();
    project.ok(&[
      "sources",
      "add",
      SOURCE,
    ]);

    project
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

  /// Runs a command expected to succeed whose output is not text.
  ///
  /// `cache cat` prints an artifact's own bytes, and an artifact is arbitrary
  /// binary, so what it prints is read as bytes rather than decoded.
  fn ok_bytes(&self, args: &[&str]) -> Vec<u8> {
    let output = self.run(args);

    assert!(
      output.status.success(),
      "`ludared {}` failed: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr)
    );

    output.stdout
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

  /// Puts a Lua codec at `relative` under `scripts/`, directories and all.
  fn write_script(&self, relative: &str) {
    self.write_codec(relative, LUA_CODEC);
  }

  /// Puts the given module at `relative` under `scripts/`, directories and all.
  fn write_codec(&self, relative: &str, source: &str) {
    let path = self.path("scripts").join(relative);

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
  }

  /// The workspace file backing an artifact, at the fixed physical path the
  /// workspace mirrors the virtual file system with.
  fn workspace_path(&self, vpath: &str) -> PathBuf {
    self.path("workspace").join(vpath)
  }

  /// Declares a decode of the source through `codec`, passing `args` to it when
  /// given one at all — an empty string being an argument like any other.
  fn decode_with_args(&self, codec: &str, args: Option<&str>) {
    let mut command = vec![
      "decode",
      "add",
      SOURCE,
      codec,
    ];

    if let Some(args) = args {
      command.push(args);
    }

    self.ok(&command);
  }

  /// The manifest as it stands on disk, read structurally rather than as text.
  fn manifest(&self) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(self.path("game.ludared")).unwrap()).unwrap()
  }

  /// The bytes the source was rebuilt into.
  fn built(&self) -> Vec<u8> {
    fs::read(self.path("builds").join(SOURCE)).unwrap()
  }

  /// What the CLI reported for a decode asking for `id` as its codec.
  fn decode_with_codec(&self, id: &str) -> String {
    let output = self.run(&[
      "decode",
      "add",
      SOURCE,
      id,
    ]);

    String::from_utf8_lossy(&output.stderr).into_owned()
  }
}

/// Asserts the registry discovered a codec under exactly `id`.
///
/// Both halves of the answer are checked, because either one alone would pass for
/// the wrong reason. A run that crashed satisfies neither claim: a crash says
/// nothing about which id the registry issued. So an id the registry never
/// handed out is caught as "Unavailable codec", and one it did hand out but whose
/// codec then failed is caught as the crash it is.
fn assert_discovered(project: &Project, id: &str) {
  let stderr = project.decode_with_codec(id);

  assert!(
    !stderr.contains("Unavailable codec"),
    "`{id}` is not an id the registry issued: {stderr}"
  );
  assert!(
    !stderr.contains("panicked"),
    "`{id}` was discovered, but the codec behind it could not run: {stderr}"
  );
}

/// Asserts the registry issued no codec under exactly `id`.
///
/// The id is echoed back by the refusal, so this turns on the id as spelled
/// rather than on the file it may or may not have come from.
fn assert_not_discovered(project: &Project, id: &str) {
  let stderr = project.fails(&[
    "decode",
    "add",
    SOURCE,
    id,
  ]);

  assert!(
    stderr.contains(&format!("Unavailable codec {id}")),
    "`{id}` should not be an id the registry issued: {stderr}"
  );
}

#[test]
fn discovers_a_lua_codec_under_the_path_it_lives_at() {
  let project = Project::with_source();
  project.write_script("sakura/text.lua");

  // The codec is named after where it sits under `scripts/`, not after anything
  // the module declares for itself: `name` is the display name, and never
  // reaches the id.
  assert_discovered(&project, SAKURA_TEXT);
}

#[test]
fn keeps_the_lua_extension_in_the_codec_id() {
  let project = Project::with_source();
  project.write_script("sakura/text.lua");

  // A codec whose name and description are those of a text table is still not a
  // codec under any id but the one carrying its extension.
  assert_discovered(&project, SAKURA_TEXT);
  assert_not_discovered(&project, SAKURA_TEXT_WITHOUT_EXTENSION);
}

#[test]
fn discovers_no_codec_under_the_shared_directories() {
  let project = Project::with_source();

  // `shared` directories are excluded from discovery, while `tests` directories
  // contain test scripts. These rules apply at any depth in the directory tree.
  project.write_script("shared/text.lua");
  project.write_script("shared/again/text.lua");
  project.write_script("tests/text.lua");
  project.write_script("tests/again/text.lua");
  project.write_script("sakura/shared/text.lua");
  project.write_script("sakura/tests/text.lua");

  for id in [
    "scripts/shared/text.lua",
    "scripts/shared/again/text.lua",
    "scripts/tests/text.lua",
    "scripts/tests/again/text.lua",
    "scripts/sakura/shared/text.lua",
    "scripts/sakura/tests/text.lua",
  ] {
    assert_not_discovered(&project, id);
  }
}

#[test]
fn still_walks_past_a_shared_directory_to_reach_a_codec() {
  let project = Project::with_source();

  // Only the directories themselves are skipped, so a codec sitting beside one
  // is still found.
  project.write_script("sakura/text.lua");
  project.write_script("shared/borrowed.lua");

  assert_discovered(&project, SAKURA_TEXT);
}

#[test]
fn accepts_a_project_with_no_scripts_directory() {
  let project = Project::with_source();

  // A project that ships no codec of its own is an ordinary one, not a broken
  // one: the absent directory is not an error, and nothing built into the
  // binary is lost for it.
  assert!(!project.path("scripts").exists());

  let listing = project.ok(&["codecs", "list"]);

  assert!(
    listing.contains(&format!("{EXTRACT}: ")),
    "`codecs list` lost the built-in codecs: {listing}"
  );
  project.ok(&[
    "decode",
    "add",
    SOURCE,
    EXTRACT,
    r#"{"name":"head.bin","offset":0,"length":5}"#,
  ]);
  assert_eq!(project.ok(&["build"]), "✓ Built builds/game.sfc\n");
}
/// A project whose only codec reports the arguments it is handed.
fn echoing_project() -> Project {
  let project = Project::with_source();

  project.write_codec("echo.lua", LUA_ECHO_ARGS);

  project
}

/// What the echoing codec was handed on the way out of `build`.
///
/// The codec rebuilds its parent as the arguments, a NUL, and the artifact, so
/// everything before the separator is whatever `encode` was given.
fn args_reaching_encode(project: &Project) -> Vec<u8> {
  project
    .built()
    .split(|byte| *byte == 0)
    .next()
    .expect("the build should begin with the arguments")
    .to_vec()
}

#[test]
fn hands_a_codec_the_same_arguments_on_encode_as_on_decode() {
  // Spacing, key order and nesting are all part of an argument string. Reading it
  // back out of the manifest has to hand `encode` the very bytes `decode` was
  // given, or a codec comparing them is comparing two different arguments.
  let args = "{ \"zebra\": 1,\n   \"apple\":   [2,  3],\n  \"nested\": {\"b\":1,\"a\":2} }";
  let project = echoing_project();

  project.decode_with_args("scripts/echo.lua", Some(args));
  project.ok(&["build"]);

  assert_eq!(args_reaching_encode(&project), args.as_bytes());
}

#[test]
fn records_arguments_that_are_not_json_at_all() {
  // Arguments are the codec's to interpret, so ludared neither needs to understand
  // them nor is allowed to refuse them. TOML, a bare number, a bare word and a
  // string holding a newline are all as good as JSON as far as it is concerned.
  for args in [
    "offset = 2, length = 4",
    "42",
    "head.bin",
    "a\nb",
  ] {
    let project = echoing_project();

    project.decode_with_args("scripts/echo.lua", Some(args));

    // Recorded as a plain string rather than parsed into anything.
    assert_eq!(
      project.manifest()["decodes"][SOURCE][0]["codec"]["args"],
      args
    );

    project.ok(&["build"]);
    assert_eq!(args_reaching_encode(&project), args.as_bytes());
  }
}

#[test]
fn keeps_an_empty_argument_apart_from_an_absent_one() {
  // `decode add` given an empty argument and given none at all are two different
  // operations, and the manifest has to hold them apart rather than treating one
  // as the other.
  let empty = echoing_project();
  empty.decode_with_args("scripts/echo.lua", Some(""));
  assert_eq!(empty.manifest()["decodes"][SOURCE][0]["codec"]["args"], "");
  empty.ok(&["build"]);
  // An empty argument reaches the codec as an empty string, so the build opens
  // with the separator itself.
  assert_eq!(args_reaching_encode(&empty), b"");

  let absent = echoing_project();
  absent.decode_with_args("scripts/echo.lua", None);
  assert_eq!(
    absent.manifest()["decodes"][SOURCE][0]["codec"]["args"],
    serde_json::Value::Null
  );
  absent.ok(&["build"]);
  // No argument at all reaches the codec as nothing, which `tostring` spells out.
  assert_eq!(args_reaching_encode(&absent), b"nil");
}

#[test]
fn a_builtin_codec_still_receives_its_arguments() {
  // Built-in codecs read their own arguments out of the string the manifest now
  // holds, so they are unaffected by it being an opaque one. A decode recorded
  // the old way and replayed the new way must still rebuild its source.
  let project = Project::with_source();

  project.ok(&[
    "decode",
    "add",
    SOURCE,
    EXTRACT,
    r#"{"name":"head.bin","offset":0,"length":5}"#,
  ]);

  // Recorded as a string, and equal to what was typed.
  assert_eq!(
    project.manifest()["decodes"][SOURCE][0]["codec"]["args"],
    r#"{"name":"head.bin","offset":0,"length":5}"#
  );

  project.ok(&["build"]);

  assert_eq!(project.built(), SOURCE_BYTES);
}

/// A project carrying two Lua codecs under `scripts/`, and one module under
/// `scripts/shared/` that is not a codec at all.
fn project_with_codecs() -> Project {
  let project = Project::new();

  project.write_codec("sakura/text.lua", LUA_CODEC);
  project.write_codec("sakura/title.lua", LUA_CODEC);
  project.write_codec("shared/helper.lua", LUA_CODEC);

  project
}

/// The codec ids a flat `codecs list` printed, in the order it printed them.
fn listed_ids(project: &Project) -> Vec<String> {
  project
    .ok(&["codecs", "list"])
    .lines()
    .map(|line| line.split_once(": ").expect("a listing line").0.to_owned())
    .collect()
}

#[test]
fn lists_the_builtin_codecs_outside_a_project() {
  let project = Project::outside_a_project();

  // No project is not a reason for the codecs built into the binary to go
  // missing, so the listing is there to be had without one.
  let ids = listed_ids(&project);

  assert!(ids.contains(&EXTRACT.to_owned()), "{ids:?}");
  assert!(
    ids.iter().all(|id| id.starts_with("std/")),
    "only built-in codecs should be listed outside a project: {ids:?}"
  );
}

#[test]
fn lists_builtin_and_lua_codecs_together_inside_a_project() {
  let project = project_with_codecs();

  let ids = listed_ids(&project);

  // The project's own codecs sit alongside the built-in ones rather than
  // replacing them.
  assert!(ids.contains(&EXTRACT.to_owned()), "{ids:?}");
  assert!(ids.contains(&SAKURA_TEXT.to_owned()), "{ids:?}");
  assert!(
    ids.contains(&"scripts/sakura/title.lua".to_owned()),
    "{ids:?}"
  );
  // A module under `shared/` is never a codec, so it is not one here either.
  assert!(
    !ids.contains(&"scripts/shared/helper.lua".to_owned()),
    "{ids:?}"
  );
}

#[test]
fn keeps_the_flat_listing_as_the_default() {
  let project = project_with_codecs();

  let flat = project.ok(&["codecs", "list"]);

  // One line per codec, id first, with the tree characters nowhere in sight.
  for line in flat.lines() {
    assert!(!line.contains("├"), "a listing line is drawn: {line}");
    assert!(!line.contains("└"), "a listing line is drawn: {line}");
  }
  assert!(
    flat.lines().any(|line| line.starts_with(SAKURA_TEXT)),
    "{flat}"
  );

  // Asking for the tree is what changes the rendering, and nothing else does.
  assert_ne!(
    flat,
    project.ok(&[
      "codecs",
      "list",
      "--tree"
    ])
  );
}

#[test]
fn orders_the_listing_the_same_way_every_time() {
  let project = project_with_codecs();

  assert_eq!(listed_ids(&project), listed_ids(&project));

  // The registry yields built-ins ahead of script codecs, which says nothing
  // about how they relate, so the listing is ordered by id rather than by where
  // each codec came from.
  let mut sorted = listed_ids(&project);
  sorted.sort();

  assert_eq!(sorted, listed_ids(&project));
}

#[test]
fn renders_the_codec_ids_as_a_tree() {
  let project = project_with_codecs();

  let tree = project.ok(&[
    "codecs",
    "list",
    "--tree",
  ]);

  // The script branch has a known structure, independent of the built-in
  // codec catalog.
  let expected_scripts = "\
scripts
└── sakura
    ├── text.lua
    └── title.lua
";

  assert!(
    tree.starts_with(expected_scripts),
    "Unexpected script tree:\n{tree}"
  );

  // Every built-in codec must also appear in the tree.
  let ids = listed_ids(&project);

  for id in ids.iter().filter(|id| id.starts_with("std/")) {
    let name = id.rsplit('/').next().unwrap();

    assert!(
      tree.lines().any(|line| {
        line.trim_start_matches([
          '│',
          '├',
          '└',
          '─',
          ' ',
        ]) == name
      }),
      "Missing codec {id} in tree:\n{tree}"
    );
  }
}

#[test]
fn reports_information_about_a_lua_codec() {
  let project = project_with_codecs();

  let info = project.ok(&[
    "codecs",
    "info",
    SAKURA_TEXT,
  ]);

  // The same three fields a built-in codec reports, and nothing script-shaped
  // among them.
  assert!(
    info.starts_with(&format!("Codec {SAKURA_TEXT}\n")),
    "{info}"
  );
  assert!(info.contains("- Name: \tMiddle byte"), "{info}");
  assert!(
    info.contains("- Description:\tExtracts the middle byte of a three-byte input."),
    "{info}"
  );
}

#[test]
fn refuses_an_id_no_codec_is_registered_under() {
  let project = project_with_codecs();

  for id in [
    // Nothing at all.
    "scripts/missing.lua",
    // A module that is not a codec, because it sits under `shared/`.
    "scripts/shared/helper.lua",
    // A built-in id without its extension.
    "std/generic/extract.lua",
  ] {
    let stderr = project.fails(&["codecs", "info", id]);

    assert!(
      stderr.contains(&format!("Unavailable codec {id}")),
      "`codecs info {id}` reported: {stderr}"
    );
  }
}

#[test]
fn decodes_the_middle_byte_and_patches_it_back_through_the_cli() {
  // A three-byte source, so the codec has exactly one byte to lift out of it.
  const MIDDLE: &str = "game.sfc/middle.bin";
  const REPLACED: &[u8] = b"\xAA\xDD\xCC";

  let project = Project::with_source_bytes(b"\xAA\xBB\xCC");
  project.write_script("sakura/text.lua");

  // The decode is declared through the CLI, which resolves the codec, asks it
  // what to call the operation, and runs it.
  project.ok(&[
    "decode",
    "add",
    SOURCE,
    SAKURA_TEXT,
  ]);

  // The middle byte came out on its own, and nothing else came out with it.
  assert_eq!(
    project.ok_bytes(&[
      "cache",
      "cat",
      MIDDLE
    ]),
    b"\xBB"
  );

  // The workspace is where an artifact is edited: overriding materializes the
  // decoded bytes, and a hand edit to them is what `build` rebuilds from.
  project.ok(&[
    "override",
    "add",
    MIDDLE,
  ]);
  fs::write(project.workspace_path(MIDDLE), b"\xDD").unwrap();

  project.ok(&["build"]);

  // `BB` went back where it came from, around the byte that replaced it.
  assert_eq!(project.built(), REPLACED);
}

#[test]
fn rebuilds_a_source_a_lua_codec_never_touched() {
  // The fixture lifts a byte out of an input of any width and puts the rest back,
  // so decoding and rebuilding a wider source is the identity on it.
  let project = Project::with_source();
  project.write_script("sakura/text.lua");

  project.ok(&[
    "decode",
    "add",
    SOURCE,
    SAKURA_TEXT,
  ]);
  project.ok(&["build"]);

  assert_eq!(project.built(), SOURCE_BYTES);
}

#[test]
fn exposes_project_paths_to_lua_codecs() {
  let project = Project::with_source();

  project.write_codec(
    "environment.lua",
    r#"
return {
  name = "Environment",
  description = "Checks the Ludared project globals.",
  version = 1,
  kind = "patch",

  can_handle = function(data) return "certain" end,
  decode_name = function(args) return "environment" end,

  decode = function(data, args)
    return {
      {
        name = "paths.txt",
        data = table.concat({
          LUDARED.PROJECT_ROOT,
          LUDARED.WORKSPACE,
          LUDARED.SCRIPTS,
        }, "\n"),
      },
    }
  end,

  encode = function(original, artifacts, args)
    return original
  end,
}
"#,
  );

  project.decode_with_args("scripts/environment.lua", None);

  let paths = project.ok_bytes(&[
    "cache",
    "cat",
    "game.sfc/paths.txt",
  ]);

  let expected = [
    ".",
    "./workspace",
    "./scripts",
  ]
  .join("\n");

  assert_eq!(paths, expected.as_bytes());
}
