use crate::project::Project;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use uuid::Uuid;

#[allow(dead_code)]
pub(crate) struct ProjectFixture {
  pub temp: TempDir,
  pub project: Project,
}

impl ProjectFixture {
  /// # Fixture lifecycle
  pub fn new() -> Self {
    let temp = TempDir::new().unwrap();

    fs::create_dir(temp.path().join("sources")).unwrap();
    fs::create_dir(temp.path().join("builds")).unwrap();

    fs::write(
      temp.path().join("ludared.toml"),
      r#"
[project]
name = "test"
manifest = "test.ludared"

[paths]
sources = "sources"
builds = "builds"
cache = "build/ludared"
"#,
    )
    .unwrap();

    fs::write(
      temp.path().join("test.ludared"),
      r#"
{
  "sources": {}
}
"#,
    )
    .unwrap();

    let project = Project::load(temp.path()).unwrap();

    Self { temp, project }
  }

  /// # Source helpers
  pub fn create_source_file(&self, name: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
    fs::write(self.project.source_path(name.as_ref()), contents).unwrap();
  }

  pub fn register_source_file(
    &mut self,
    source_name: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
  ) {
    let source_name = source_name.as_ref();

    self.create_source_file(source_name, contents);
    self.project.add_source(source_name).unwrap();
    self.reload();

    assert!(self.project.manifest.sources.contains_key(source_name));
  }

  /// # Project helpers
  pub fn reload(&mut self) {
    self.project = Project::load(self.temp.path()).unwrap();
  }

  pub fn random_source_name() -> (String, PathBuf) {
    let file = format!("{}.sfc", Uuid::new_v4());
    let path = PathBuf::from(&file);

    (file, path)
  }
}
