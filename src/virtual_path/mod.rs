use std::path::PathBuf;

use crate::project::Project;

use errors::VirtualPathError;
pub(crate) mod errors;

/// A logical path identifying a source or a derived artifact within a project.
///
/// Virtual paths are relative, use `/` as their separator regardless of the
/// host platform, and always start with a source file.
#[derive(Debug, PartialEq, Eq)]
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

  /// Returns whether this virtual path starts with `prefix` as plain text.
  ///
  /// Unlike [`Self::is_within`], this compares the path character by character
  /// and knows nothing of `/` being a component boundary, which is what makes it
  /// useful for completion: it is the answer to "does what has been typed so far
  /// still match this path", including the half-typed component at its end.
  ///
  /// ```
  /// # use crate::virtual_path::VirtualPath;
  /// let vpath = VirtualPath::new("DBZ.sfc/rom_00.bin").unwrap();
  /// assert!(vpath.has_text_prefix("DBZ"));
  /// assert!(vpath.has_text_prefix("DBZ.sfc/rom"));
  /// assert!(!vpath.has_text_prefix("rom_00.bin"));
  /// ```
  pub fn has_text_prefix(&self, prefix: &str) -> bool {
    self.path.starts_with(prefix)
  }

  /// Returns this virtual path and every path leading to it, shortest first.
  ///
  /// `DBZ.sfc/rom_00.bin` yields `DBZ.sfc`, then `DBZ.sfc/rom_00.bin`. The
  /// intermediate paths are not artifacts of their own, but they still name a
  /// branch of the virtual file system, which is what makes them worth asking
  /// about.
  pub fn ancestors(&self) -> impl Iterator<Item = VirtualPath> {
    let mut prefix = String::new();

    self.components().map(move |component| {
      prefix = match prefix.is_empty() {
        true => component.to_owned(),
        false => format!("{prefix}/{component}"),
      };

      VirtualPath {
        path: prefix.clone(),
      }
    })
  }

  /// Returns whether this virtual path is `root` itself or one of its
  /// descendants.
  ///
  /// Components are compared whole, so `DBZ.sfc/rom_00` never contains
  /// `DBZ.sfc/rom_00.bin` and `DBZ.sfc` contains nothing named `DBZ`. This is
  /// what lets a path stand for a whole branch of the virtual file system, which
  /// is what filtering a listing by a path means.
  ///
  /// ```
  /// # use crate::virtual_path::VirtualPath;
  /// let vpath = VirtualPath::new("DBZ.sfc/rom_00.bin").unwrap();
  /// let root = VirtualPath::new("DBZ.sfc").unwrap();
  /// let partial = VirtualPath::new("DBZ.sfc/rom_00").unwrap();
  /// assert!(vpath.is_within(&root));
  /// assert!(!vpath.is_within(&partial));
  /// ```
  pub fn is_within(&self, root: &VirtualPath) -> bool {
    let mut components = self.components();

    root
      .components()
      .all(|component| components.next() == Some(component))
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

  #[test]
  fn matches_text_as_typed_however_deep_it_sits() {
    // What makes this useful for completion: the prefix is text, not a path, and
    // may stop anywhere at all, including inside a component.
    for path in [
      "DBZ.sfc",
      "DBZ.sfc/rom_00.bin",
      "DBZ.sfc/rom_00.bin/ROM_NAME.txt",
    ] {
      let vpath = VirtualPath::new(path).unwrap();

      assert!(vpath.has_text_prefix("DBZ"), "'{path}' should match 'DBZ'");
      assert!(vpath.has_text_prefix("DBZ.sfc"), "'{path}' should match");
      assert!(vpath.has_text_prefix(path), "'{path}' should match itself");
    }
  }

  #[test]
  fn does_not_match_text_that_merely_appears_later() {
    // A component that only looks like the prefix does not carry it.
    let vpath = VirtualPath::new("Other.sfc/DBZ.bin").unwrap();

    assert!(!vpath.has_text_prefix("DBZ"));
    assert!(!vpath.has_text_prefix("DBZ.bin"));
    assert!(!vpath.has_text_prefix("Other.sfc/DBZ.bak"));
  }

  #[test]
  fn ignores_component_boundaries_where_text_has_none() {
    // This is the point of the two methods differing: `DBZ` is the whole of a
    // component here, but the text still carries on into `DBZ.bin`, so a
    // half-typed word keeps matching.
    let vpath = VirtualPath::new("Other.sfc/DBZ.bin").unwrap();

    assert!(vpath.has_text_prefix("Other.sfc/DBZ"));
    assert!(!vpath.has_text_prefix("Other.sfc/DBZ.bin/rom_00"));
  }

  #[test]
  fn matches_a_path_within_itself_and_its_descendants() {
    let root = VirtualPath::new("DBZ.sfc").unwrap();

    assert!(VirtualPath::new("DBZ.sfc").unwrap().is_within(&root));
    assert!(
      VirtualPath::new("DBZ.sfc/rom_00.bin")
        .unwrap()
        .is_within(&root)
    );
    assert!(
      VirtualPath::new("DBZ.sfc/rom_00.bin/ROM_NAME.txt")
        .unwrap()
        .is_within(&root)
    );
  }

  #[test]
  fn does_not_match_a_path_that_ends_where_the_root_begins() {
    let root = VirtualPath::new("DBZ.sfc").unwrap();

    assert!(!VirtualPath::new("DBZ").unwrap().is_within(&root));
    assert!(
      !VirtualPath::new("Other.sfc/DBZ.bin")
        .unwrap()
        .is_within(&root)
    );
    assert!(
      !VirtualPath::new("DBZ.sfc.bak/rom_00.bin")
        .unwrap()
        .is_within(&root)
    );
  }

  #[test]
  fn matches_components_whole_rather_than_as_text() {
    // `rom_00` is not the component `rom_00.bin`, however its text reads, and
    // `DBZ` is not the component `DBZ.sfc`.
    let vpath = VirtualPath::new("DBZ.sfc/rom_00.bin").unwrap();

    assert!(!vpath.is_within(&VirtualPath::new("DBZ.sfc/rom_00").unwrap()));
    assert!(!vpath.is_within(&VirtualPath::new("DBZ").unwrap()));
    assert!(!vpath.is_within(&VirtualPath::new("DBZ.sfc/rom_00.b").unwrap()));
    assert!(vpath.is_within(&VirtualPath::new("DBZ.sfc/rom_00.bin").unwrap()));
  }

  #[test]
  fn tells_textual_and_hierarchical_matching_apart() {
    // The same pair of paths, asked two different questions.
    let partial = VirtualPath::new("DBZ.sfc/rom_00").unwrap();
    let complete = VirtualPath::new("DBZ.sfc/rom_00.bin").unwrap();

    assert!(complete.has_text_prefix("DBZ.sfc/rom_00"));
    assert!(!complete.is_within(&partial));

    assert!(partial.has_text_prefix("DBZ.sfc/rom"));
    assert!(!complete.has_text_prefix("DBZ.sfc/rom_00/tail"));
  }

  #[test]
  fn yields_a_path_and_the_ones_leading_to_it() {
    let ancestors = |path: &str| {
      VirtualPath::new(path)
        .unwrap()
        .ancestors()
        .map(|vpath| vpath.to_string())
        .collect::<Vec<String>>()
    };

    assert_eq!(ancestors("DBZ.sfc"), ["DBZ.sfc"]);
    assert_eq!(
      ancestors("DBZ.sfc/rom_00.bin"),
      [
        "DBZ.sfc",
        "DBZ.sfc/rom_00.bin"
      ]
    );
    assert_eq!(
      ancestors("DBZ.sfc/rom_00.bin/ROM_NAME.txt"),
      [
        "DBZ.sfc",
        "DBZ.sfc/rom_00.bin",
        "DBZ.sfc/rom_00.bin/ROM_NAME.txt"
      ]
    );
  }

  #[test]
  fn yields_whole_components_whatever_the_names_look_like() {
    // The same pair that `is_within` refuses: a path is not below the component
    // `rom_00` any more than it is below `DBZ`.
    let ancestors = |path: &str| {
      VirtualPath::new(path)
        .unwrap()
        .ancestors()
        .map(|vpath| vpath.to_string())
        .collect::<Vec<String>>()
    };

    assert_eq!(
      ancestors("DBZ.sfc/rom_00.bin"),
      [
        "DBZ.sfc",
        "DBZ.sfc/rom_00.bin"
      ]
    );
    assert!(!ancestors("DBZ.sfc/rom_00.bin").contains(&"DBZ.sfc/rom_00".to_owned()));
    assert!(!ancestors("DBZ.sfc/rom_00.bin").contains(&"DBZ".to_owned()));
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
