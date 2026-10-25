use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

pub(crate) const RESERVED_DIRECTORIES: &[&str] = &["shared", "tests"];
pub(crate) const SCRIPT_EXTENSIONS: &[&str] = &["lua"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptKind {
  Codec,
  Test,
}

#[derive(Debug)]
struct DiscoveredScript {
  path: PathBuf,
  kind: ScriptKind,
}

#[derive(Debug)]
pub(crate) struct ScriptDiscovery {
  root: PathBuf,
  scripts: Vec<DiscoveredScript>,
}

impl ScriptDiscovery {
  pub(crate) fn new(root: PathBuf) -> io::Result<Self> {
    let mut discovery = Self {
      root,
      scripts: Vec::new(),
    };

    match std::fs::metadata(&discovery.root) {
      Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
        return Ok(discovery);
      }
      Err(err) => return Err(err),
      Ok(_) => (),
    }

    discovery.scan(&discovery.root.clone(), ScriptKind::Codec)?;

    Ok(discovery)
  }

  fn scan(&mut self, directory: &Path, kind: ScriptKind) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
      let entry = entry?;
      let path = entry.path();
      let file_type = entry.file_type()?;

      if file_type.is_dir() {
        match entry.file_name().to_str() {
          Some("shared") => continue,
          Some("tests") => self.scan(&path, ScriptKind::Test)?,
          _ => self.scan(&path, kind)?,
        }

        continue;
      }

      if file_type.is_file() {
        if !path
          .extension()
          .and_then(|ext| ext.to_str())
          .is_some_and(|ext| SCRIPT_EXTENSIONS.contains(&ext))
        {
          continue;
        }
        self.scripts.push(DiscoveredScript {
          path: path.strip_prefix(&self.root).unwrap().to_path_buf(),
          kind,
        });
      }
    }

    Ok(())
  }

  pub(crate) fn retrieve_scripts(&self, kind: ScriptKind) -> Vec<&Path> {
    self
      .scripts
      .iter()
      .filter(|script| script.kind == kind)
      .map(|script| script.path.as_path())
      .collect()
  }
}
