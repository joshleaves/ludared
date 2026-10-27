use std::collections::BTreeSet;
use std::iter::Peekable;
use std::str::Chars;

use super::*;

enum CharClassToken {
  Literal(char),
  Range(char, char),
  Close(bool),
}

impl CharClassToken {
  fn to_chars(&self) -> Vec<char> {
    match self {
      Self::Close(_) => vec![],
      Self::Literal(ch) => vec![*ch],
      Self::Range(start, end) => (*start..=*end).collect(),
    }
  }
}

impl GlobToken {
  pub(super) fn parse_char_class(chars_iter: &mut Peekable<Chars<'_>>) -> Option<Self> {
    let checkpoint = chars_iter.clone();

    let mut tokens: Vec<CharClassToken> = Vec::new();

    while let Some(next_ch) = chars_iter.next() {
      if next_ch == ']' {
        let token = CharClassToken::Close(chars_iter.next_if_eq(&'+').is_some());
        tokens.push(token);
        break;
      }

      let inner_checkpoint = chars_iter.clone();

      let next_ch = Self::parse_backslash(chars_iter, next_ch);
      if chars_iter.next_if_eq(&'-').is_none() {
        tokens.push(CharClassToken::Literal(next_ch));
      } else {
        let chr_end = match chars_iter.next() {
          None => {
            *chars_iter = checkpoint;
            return None;
          }
          Some(chr_end) => chr_end,
        };
        let chr_escape = chr_end == '\\';
        let chr_end = Self::parse_backslash(chars_iter, chr_end);
        if chr_end == ']' && !chr_escape {
          tokens.push(CharClassToken::Literal(next_ch));
          *chars_iter = inner_checkpoint;
        } else {
          tokens.push(CharClassToken::Range(next_ch, chr_end));
        }
      }
    }

    // Faaaaail
    let Some(CharClassToken::Close(repeat)) = tokens.last() else {
      *chars_iter = checkpoint;
      return None;
    };

    let chars: BTreeSet<char> = tokens.iter().flat_map(CharClassToken::to_chars).collect();
    Some(GlobToken::CharClass(chars.into_iter().collect(), *repeat))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse(pattern: &str) -> (Option<GlobToken>, String) {
    let mut chars = pattern.chars().peekable();
    assert_eq!(chars.next(), Some('['));

    let token = GlobToken::parse_char_class(&mut chars);
    let remaining = chars.collect();

    (token, remaining)
  }

  fn assert_class(pattern: &str, expected: &str, repeated: bool) {
    let (token, remaining) = parse(pattern);

    match token {
      Some(GlobToken::CharClass(chars, actual_repeated)) => {
        let actual: BTreeSet<_> = chars.into_iter().collect();
        let expected: BTreeSet<_> = expected.chars().collect();

        assert_eq!(actual, expected, "Pattern: {pattern}");
        assert_eq!(actual_repeated, repeated, "Pattern: {pattern}");
        assert!(remaining.is_empty(), "Remaining: {remaining}");
      }
      _ => panic!("Expected CharClass for {pattern}"),
    }
  }

  #[test]
  fn char_class_literals() {
    assert_class("[abc]", "abc", false);
    assert_class("[a]", "a", false);
    assert_class("[aabbcc]", "abc", false);
    assert_class("[123]", "123", false);
  }

  #[test]
  fn char_class_ranges() {
    assert_class("[a-z]", "abcdefghijklmnopqrstuvwxyz", false);
    assert_class("[0-9]", "0123456789", false);
    assert_class("[A-C]", "ABC", false);
    assert_class("[a-a]", "a", false);
  }

  #[test]
  fn char_class_mixed() {
    assert_class("[a-c0-2]", "abc012", false);
    assert_class("[a-cXYZ]", "abcXYZ", false);
    assert_class("[abc0-2]", "abc012", false);
    assert_class("[0-2a-c_]", "012abc_", false);
  }

  #[test]
  fn char_class_repeated() {
    assert_class("[0-9]+", "0123456789", true);
    assert_class("[abc]+", "abc", true);
    assert_class("[a-z]+", "abcdefghijklmnopqrstuvwxyz", true);
  }

  #[test]
  fn char_class_escaped() {
    assert_class(r"[\]]", "]", false);
    assert_class(r"[\-]", "-", false);
    assert_class(r"[\\]", "\\", false);
    assert_class(r"[a\-z]", "a-z", false);
    assert_class(r"[[-\]]", "[\\]", false);
  }

  #[test]
  fn char_class_hyphens() {
    assert_class("[a-]", "a-", false);
    assert_class("[-a]", "-a", false);
    assert_class("[-]", "-", false);
  }

  #[test]
  fn char_class_followed_by_text() {
    let (token, remaining) = parse("[0-9]+.bin");

    assert!(matches!(token, Some(GlobToken::CharClass(_, true))));
    assert_eq!(remaining, ".bin");
  }

  #[test]
  fn char_class_incomplete() {
    for pattern in [
      "[abc",
      "[a-z",
      "[",
      "[a-",
    ] {
      let (token, remaining) = parse(pattern);

      assert!(token.is_none(), "Pattern: {pattern}");
      assert_eq!(remaining, &pattern[1..], "Pattern: {pattern}");
    }
  }
}
