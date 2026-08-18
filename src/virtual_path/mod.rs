use std::path::PathBuf;

use crate::project::Project;

use errors::VirtualPathError;
pub(crate) mod errors;

/// A logical path identifying a source or a derived artifact within a project.
///
/// Virtual paths are relative, use `/` as their separator regardless of the
/// host platform, and always start with a source file.
pub(crate) struct VirtualPath {
  path: String,
}

impl VirtualPath {
  /// Parses and validates a virtual path.
  pub fn new(path: &str) -> Result<Self, VirtualPathError> {
    if path.is_empty() {
      return Err(VirtualPathError::EmptyPath);
    }
    if path.starts_with('/') {
      return Err(VirtualPathError::StartsWithSlash(path.to_owned()));
    }
    if path.ends_with('/') {
      return Err(VirtualPathError::EndsWithSlash(path.to_owned()));
    }
    if path.split('/').any(|c| c.is_empty()) {
      return Err(VirtualPathError::EmptyComponent(path.to_owned()));
    }
    if path.split('/').any(|c| c == "." || c == "..") {
      return Err(VirtualPathError::ContainsReferenceComponent(
        path.to_owned(),
      ));
    }
    if path.contains(r"\") {
      return Err(VirtualPathError::ContainsBackslash(path.to_owned()));
    }

    Ok(Self {
      path: path.to_owned(),
    })
  }

  pub fn resolve(&self, project: &Project) -> Result<PathBuf, VirtualPathError> {
    if self.is_source() {
      let path = project.configuration.paths.sources.join(&self.path);
      if !path.exists() {
        return Err(VirtualPathError::MissingFile(path));
      }
      return Ok(path);
    }
    let Some(path) = project.cache.get_entry(self) else {
      unimplemented!("TODO: Missing file in Manifest");
    };
    if !path.exists() {
      return Err(VirtualPathError::MissingFile(path));
    }
    Ok(path)
  }

  /// Returns a new virtual path with `component` appended.
  pub fn join(&self, component: &str) -> Result<Self, VirtualPathError> {
    Self::new(
      &[
        &self.path,
        component,
      ]
      .join("/"),
    )
  }

  // /// Returns the parent virtual path, or `None` if this path identifies a source.
  // pub fn parent(&self) -> Option<Self>;

  // /// Returns the final component of this virtual path.
  // pub fn filename(&self) -> &str;

  /// Returns an iterator over the components of this virtual path.
  pub fn components(&self) -> impl Iterator<Item = &str> {
    self.path.split('/')
  }

  // pub fn consume(&mut self, components: Vec<&str>) {
  //   // TODO: An error here at some point
  //   let path: Vec<&str> = self.path.split('/').collect();

  //   if path.starts_with(&components) {
  //     self.path = path[components.len()..].join("/");
  //   }
  // }

  /// Returns the number of components in this virtual path.
  pub fn depth(&self) -> usize {
    self.path.split('/').count()
  }

  // /// Returns whether this virtual path directly identifies a project source.
  pub fn is_source(&self) -> bool {
    self.depth() == 1
  }
}

impl std::fmt::Display for VirtualPath {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.path)
  }
}

#[cfg(test)]
mod tests {
  use uuid::Uuid;

  use super::*;
  use crate::testing::fixtures::project::ProjectFixture;

  #[test]
  fn accepts_source_path() {
    let (file, _) = ProjectFixture::random_source_name();
    let path = VirtualPath::new(&file).unwrap();

    assert_eq!(path.path, file);
    assert_eq!(path.depth(), 1);
    assert!(path.is_source());
  }

  #[test]
  fn accepts_nested_virtual_path() {
    let file = format!(
      "{}.sfc/rom_00.bin/title_screen.bmp",
      ProjectFixture::random_source_name().0
    );
    let path = VirtualPath::new(&file).unwrap();

    assert_eq!(path.path, file);
    assert_eq!(path.depth(), 3);
    assert!(!path.is_source());
  }

  #[test]
  fn rejects_empty_path() {
    assert!(matches!(
      VirtualPath::new(""),
      Err(VirtualPathError::EmptyPath)
    ));
  }

  #[test]
  fn rejects_path_starting_with_slash() {
    let file = format!("/{}.sfc", Uuid::new_v4());
    assert!(matches!(
      VirtualPath::new(&file),
      Err(VirtualPathError::StartsWithSlash(path))
        if path == file
    ));
  }

  #[test]
  fn rejects_path_ending_with_slash() {
    let file = format!("{}.sfc/", Uuid::new_v4());
    assert!(matches!(
      VirtualPath::new(&file),
      Err(VirtualPathError::EndsWithSlash(path))
        if path == file
    ));
  }

  #[test]
  fn rejects_empty_component() {
    let file = format!("{}.sfc//rom_00.bin", Uuid::new_v4());
    assert!(matches!(
      VirtualPath::new(&file),
      Err(VirtualPathError::EmptyComponent(path))
        if path == file
    ));
  }

  #[test]
  fn rejects_current_directory_component() {
    let file = format!("{}.sfc/./rom_00.bin", Uuid::new_v4());
    assert!(matches!(
      VirtualPath::new(&file),
      Err(VirtualPathError::ContainsReferenceComponent(path))
        if path == file
    ));
  }

  #[test]
  fn rejects_parent_directory_component() {
    let file = format!("{}.sfc/../rom_00.bin", Uuid::new_v4());
    assert!(matches!(
      VirtualPath::new(&file),
      Err(VirtualPathError::ContainsReferenceComponent(path))
        if path == file
    ));
  }

  #[test]
  fn rejects_backslash() {
    let file = format!(r"{}.sfc\rom_00.bin", Uuid::new_v4());
    assert!(matches!(
      VirtualPath::new(&file),
      Err(VirtualPathError::ContainsBackslash(path))
        if path == file
    ));
  }
}
