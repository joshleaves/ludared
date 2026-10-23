use super::*;
use std::iter::Peekable;
use std::str::Chars;

struct WordAccumulator(Vec<char>);
impl WordAccumulator {
  fn consume(&mut self) -> String {
    let word: String = self.0.iter().collect();
    self.0.clear();
    word
  }
}

impl GlobToken {
  pub(super) fn parse_alternatives(chars_iter: &mut Peekable<Chars<'_>>) -> Option<Self> {
    let checkpoint = chars_iter.clone();
    let mut words: Vec<String> = Vec::new();
    let mut current_word = WordAccumulator(Vec::new());
    let mut closed = false;

    while let Some(next_ch) = chars_iter.next() {
      match next_ch {
        '}' => {
          closed = true;
          words.push(current_word.consume());
          break;
        }
        ',' => words.push(current_word.consume()),
        chr => {
          let chr = Self::parse_backslash(chars_iter, chr);
          current_word.0.push(chr);
        }
      }
    }
    if !closed {
      *chars_iter = checkpoint;
      return None;
    }

    Some(GlobToken::Alternatives(words))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse(pattern: &str) -> (Option<GlobToken>, String) {
    let mut chars = pattern.chars().peekable();
    assert_eq!(chars.next(), Some('{'));

    let token = GlobToken::parse_alternatives(&mut chars);
    let remaining: String = chars.collect();

    (token, remaining)
  }

  fn assert_alternatives(pattern: &str, expected: &[&str]) {
    let (token, remaining) = parse(pattern);

    match token {
      Some(GlobToken::Alternatives(words)) => {
        assert_eq!(words, expected, "Pattern: {pattern}");
        assert!(remaining.is_empty(), "Remaining: {remaining}");
      }
      _ => panic!("Expected Alternatives for {pattern}"),
    }
  }

  #[test]
  fn alternatives_basic() {
    assert_alternatives("{foo,bar}", &["foo", "bar"]);
    assert_alternatives("{foo,bar,baz}", &["foo", "bar", "baz"]);
    assert_alternatives("{a,b}", &["a", "b"]);
    assert_alternatives("{hello,world}", &["hello", "world"]);
  }

  #[test]
  fn alternatives_escaped() {
    assert_alternatives(r"{foo\,bar,baz}", &["foo,bar", "baz"]);
    assert_alternatives(r"{foo\},bar}", &["foo}", "bar"]);
    assert_alternatives(r"{foo\{,bar}", &["foo{", "bar"]);
    assert_alternatives(r"{foo\\,bar}", &["foo\\", "bar"]);
    assert_alternatives(r"{foo\,,bar}", &["foo,", "bar"]);
  }

  #[test]
  fn alternatives_empty_words() {
    assert_alternatives("{,foo}", &["", "foo"]);
    assert_alternatives("{foo,}", &["foo", ""]);
    assert_alternatives("{foo,,bar}", &["foo", "", "bar"]);
    assert_alternatives("{,}", &["", ""]);
  }

  #[test]
  fn alternatives_duplicates() {
    assert_alternatives("{foo,foo}", &["foo", "foo"]);
    assert_alternatives("{a,b,a}", &["a", "b", "a"]);
  }

  #[test]
  fn alternatives_followed_by_text() {
    let (token, remaining) = parse("{foo,bar}.bin");

    match token {
      Some(GlobToken::Alternatives(words)) => {
        assert_eq!(words, ["foo", "bar"]);
      }
      _ => panic!("Expected Alternatives"),
    }

    assert_eq!(remaining, ".bin");
  }

  #[test]
  fn alternatives_followed_by_wildcard() {
    let (token, remaining) = parse("{foo,bar}*.bin");

    assert!(matches!(token, Some(GlobToken::Alternatives(_))));
    assert_eq!(remaining, "*.bin");
  }

  #[test]
  fn alternatives_incomplete() {
    for pattern in [
      "{foo,bar",
      "{foo",
      "{",
      "{foo\\}",
    ] {
      let (token, remaining) = parse(pattern);

      assert!(token.is_none(), "Pattern: {pattern}");
      assert_eq!(remaining, &pattern[1..], "Pattern: {pattern}");
    }
  }

  #[test]
  fn alternatives_single_word() {
    assert_alternatives("{foo}", &["foo"]);
  }

  #[test]
  fn alternatives_unicode() {
    assert_alternatives("{été,hiver}", &["été", "hiver"]);
    assert_alternatives("{日本語,français}", &["日本語", "français"]);
  }
}
