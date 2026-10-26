//! Check trace export against real builtin and scripted codec executions.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

const BUILTIN: &str = "std/generic/extract";
const ARGS: &str = r#"{"name":"part.bin","offset":1,"length":2}"#;
const SCRIPT: &str = r#"
local codec = LUDARED.codec("std/generic/extract")
return {
  name = "Delegating codec",
  description = "Exercises nested codec calls",
  version = 1,
  kind = "patch",
  can_handle = function(data) return "certain" end,
  decode_name = function(args) return "part" end,
  decode = function(data, args) return codec:decode(data, args) end,
  encode = function(original, artifacts, args)
    return codec:encode(original, artifacts, args)
  end,
}
"#;

fn run(dir: &Path, args: &[&str], success: bool) -> Vec<Value> {
  let output = Command::new(env!("CARGO_BIN_EXE_ludared"))
    .current_dir(dir)
    .args(args)
    .output()
    .unwrap();
  assert_eq!(
    output.status.success(),
    success,
    "{args:?}: {}",
    String::from_utf8_lossy(&output.stderr)
  );

  let traces: Vec<_> = fs::read_dir(dir)
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .filter(|path| {
      path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("trace-")
        && path.extension().is_some_and(|ext| ext == "json")
    })
    .collect();
  assert_eq!(traces.len(), usize::from(cfg!(feature = "profiling")));

  let mut events = Vec::new();
  for trace in traces {
    events.extend(serde_json::from_slice::<Vec<Value>>(&fs::read(&trace).unwrap()).unwrap());
    fs::remove_file(trace).unwrap();
  }
  events
}

fn assert_spans(events: &[Value], codec: &str, operation: &str) {
  if !cfg!(feature = "profiling") {
    assert!(events.is_empty());
    return;
  }

  let spans: Vec<_> = events
    .iter()
    .filter(|event| event["ph"] == "B" || event["ph"] == "E")
    .collect();
  let expected = if codec == BUILTIN {
    vec![
      ("B", codec),
      ("E", codec),
    ]
  } else {
    vec![
      ("B", codec),
      ("B", BUILTIN),
      ("E", BUILTIN),
      ("E", codec),
    ]
  };
  assert_eq!(spans.len(), expected.len(), "{events:?}");
  for (event, (phase, id)) in spans.iter().zip(expected) {
    assert_eq!(event["ph"], phase);
    assert_eq!(event["name"], operation);
    assert_eq!(event["args"]["codec_id"], id);
    assert_eq!(event["args"]["operation"], operation);
    assert!(event["ts"].is_number());
  }
}

#[test]
fn profiles_builtin_and_unique_script_ids_only_when_enabled() {
  for codec in [
    BUILTIN,
    "scripts/first.lua",
    "scripts/second.lua",
  ] {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
      root.join("ludared.toml"),
      r#"
[project]
name = "profiling-test"
manifest = "game.ludared"
[paths]
sources = "sources"
scripts = "scripts"
workspace = "workspace"
builds = "builds"
cache = "builds/cache"
"#,
    )
    .unwrap();
    fs::write(root.join("game.ludared"), r#"{"sources":{}}"#).unwrap();
    fs::create_dir(root.join("sources")).unwrap();
    fs::create_dir(root.join("scripts")).unwrap();
    fs::write(root.join("sources/input.bin"), b"abcd").unwrap();
    for name in [
      "first.lua",
      "second.lua",
    ] {
      fs::write(root.join("scripts").join(name), SCRIPT).unwrap();
    }

    run(
      root,
      &[
        "sources",
        "add",
        "input.bin",
      ],
      true,
    );
    assert_spans(
      &run(
        root,
        &[
          "decode",
          "add",
          "input.bin",
          codec,
          ARGS,
        ],
        true,
      ),
      codec,
      "decode",
    );
    assert_spans(&run(root, &["unpack"], true), codec, "decode");
    assert_spans(&run(root, &["build"], true), codec, "encode");
    assert_eq!(fs::read(root.join("builds/input.bin")).unwrap(), b"abcd");

    // Even an error must close the nested spans and flush valid trace JSON.
    assert_spans(
      &run(
        root,
        &[
          "decode",
          "add",
          "input.bin",
          codec,
          r#"{"name":"bad.bin","offset":100,"length":2}"#,
        ],
        false,
      ),
      codec,
      "decode",
    );
  }
}
