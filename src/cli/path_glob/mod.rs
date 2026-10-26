use std::iter::Peekable;
use std::str::Chars;

pub mod component_glob;
use component_glob::ComponentGlob;

enum PathComponent {
  Normal(ComponentGlob),
  DoubleStar,
}

pub(crate) struct PathGlob {
  components: Vec<PathComponent>,
}

enum GlobToken {
  Literal(char),
  AnyChar,                    // ?
  AnySequence,                // *
  CharClass(Vec<char>, bool), // [0-9]
  Alternatives(Vec<String>),  // {a,b}
}

impl GlobToken {
  pub(super) fn parse_backslash(chars_iter: &mut Peekable<Chars<'_>>, chr: char) -> char {
    match chr {
      '\\' => chars_iter.next().unwrap_or('\\'),
      chr => chr,
    }
  }
}

impl PathGlob {
  pub(crate) fn new(pattern: &str) -> Self {
    let components = pattern
      .split('/')
      .fold(Vec::new(), |mut components, component| {
        if component == "**" {
          if !matches!(components.last(), Some(PathComponent::DoubleStar)) {
            components.push(PathComponent::DoubleStar);
          }
        } else {
          components.push(PathComponent::Normal(ComponentGlob::parse(component)));
        }

        components
      });

    Self { components }
  }

  pub(crate) fn is_glob(&self) -> bool {
    self.components.iter().any(|component| match component {
      PathComponent::Normal(component_glob) => component_glob
        .tokens
        .iter()
        .any(|token| !matches!(token, GlobToken::Literal(_))),
      PathComponent::DoubleStar => true,
    })
  }

  pub(crate) fn matches(&self, path: &str) -> bool {
    let path_components: Vec<&str> = path.split('/').collect();
    let mut stack = vec![(0, 0)];

    while let Some((pattern_idx, path_idx)) = stack.pop() {
      if pattern_idx == self.components.len() {
        if path_idx == path_components.len() {
          return true;
        }
        continue;
      }

      match &self.components[pattern_idx] {
        PathComponent::Normal(glob) => {
          if path_idx < path_components.len() && glob.matches(path_components[path_idx]) {
            stack.push((pattern_idx + 1, path_idx + 1));
          }
        }
        PathComponent::DoubleStar => {
          stack.push((pattern_idx + 1, path_idx));

          if path_idx < path_components.len() {
            stack.push((pattern_idx, path_idx + 1));
          }
        }
      }
    }

    false
  }
}

#[cfg(test)]
mod tests {
  use crate::cli::path_glob::PathGlob;

  fn assert_path_matches(pattern: &str, matching: &[&str], not_matching: &[&str]) {
    let glob = PathGlob::new(pattern);

    for candidate in matching {
      assert!(
        glob.matches(candidate),
        "{pattern:?} should match {candidate:?}"
      );
    }

    for candidate in not_matching {
      assert!(
        !glob.matches(candidate),
        "{pattern:?} should not match {candidate:?}"
      );
    }
  }

  #[test]
  fn recursive_at_start() {
    assert_path_matches(
      "**/file.txt",
      &[
        "file.txt",
        "dir/file.txt",
        "dir/a/b/c/file.txt",
      ],
      &[
        "",
        "dir/",
        "dir/file.txt.bak",
        "dir/x/file.txt/extra",
      ],
    );
  }

  #[test]
  fn recursive_at_end() {
    assert_path_matches(
      "dir/**",
      &[
        "dir",
        "dir/file.txt",
        "dir/a/b/c",
        "dir/file.txt/",
      ],
      &[
        "",
        "dirfile.txt",
        "dirfile.txt/extra",
      ],
    );
  }

  #[test]
  fn recursive_in_middle() {
    assert_path_matches(
      "src/**/file.txt",
      &[
        "src/file.txt",
        "src/a/file.txt",
        "src/a/b/c/file.txt",
      ],
      &[
        "",
        "src/",
        "srcfile.txt",
        "srcfile.txt/extra",
        "a/src/file.txt",
      ],
    );
  }
}
