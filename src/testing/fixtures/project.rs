use crate::project::Project;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(dead_code)]
pub(crate) struct ProjectFixture {
  pub temp: TempDir,
  pub project: Project,
}

impl ProjectFixture {
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

  // #[allow(dead_code)]
  pub fn create_source_file(&self, name: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
    fs::write(self.project.source_path(name.as_ref()), contents).unwrap();
  }

  pub fn reload(&mut self) -> &mut Self {
    self.project = Project::load(self.temp.path()).unwrap();
    self
  }
}
