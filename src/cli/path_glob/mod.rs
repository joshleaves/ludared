use std::iter::Peekable;
use std::str::Chars;

pub mod component_glob;
use component_glob::ComponentGlob;

use crate::cli::path_glob::GlobToken::Literal;

pub(crate) struct PathGlob {
  components: Vec<ComponentGlob>,
}

enum GlobToken {
  Literal(char),
  AnyChar,                    // ?
  AnySequence,                // *
  CharClass(Vec<char>, bool), // [0-9]
  Alternatives(Vec<String>),  // {a,b}
}

impl PathGlob {
  pub(crate) fn new(pattern: &str) -> Self {
    let components = pattern
      .split('/')
      .map(ComponentGlob::parse)
      .collect::<Vec<_>>();

    Self { components }
  }

  pub(crate) fn is_glob(&self) -> bool {
    self.components.iter().any(|component| {
      component
        .tokens
        .iter()
        .any(|token| !matches!(token, Literal(_)))
    })
  }

  pub(crate) fn matches(&self, path: &str) -> bool {
    let components = path.split('/').collect::<Vec<_>>();

    if components.len() != self.components.len() {
      return false;
    }

    self
      .components
      .iter()
      .zip(components)
      .all(|(pattern, component)| pattern.matches(component))
  }
}

impl GlobToken {
  pub(super) fn parse_backslash(chars_iter: &mut Peekable<Chars<'_>>, chr: char) -> char {
    match chr {
      '\\' => chars_iter.next().unwrap_or('\\'),
      chr => chr,
    }
  }
}
