pub(super) mod alternatives;
pub(super) mod char_class;
pub mod consumer;

use super::GlobToken;
use consumer::ComponentConsumer;

pub(super) struct ComponentGlob {
  pub(super) tokens: Vec<GlobToken>,
}

impl ComponentGlob {
  pub(super) fn parse(pattern: &str) -> Self {
    let mut chars = pattern.chars().peekable();
    let mut tokens = Vec::new();

    while let Some(ch) = chars.next() {
      let token = match ch {
        '\\' => match chars.next() {
          Some(ch) => GlobToken::Literal(ch),
          None => GlobToken::Literal('\\'),
        },
        '*' => GlobToken::AnySequence,
        '?' => GlobToken::AnyChar,
        '[' => match GlobToken::parse_char_class(&mut chars) {
          Some(tok) => tok,
          None => GlobToken::Literal(ch),
        },
        '{' => match GlobToken::parse_alternatives(&mut chars) {
          Some(tok) => tok,
          None => GlobToken::Literal(ch),
        },
        _ => GlobToken::Literal(ch),
      };

      tokens.push(token);
    }

    Self { tokens }
  }

  pub(super) fn matches(&self, component: &str) -> bool {
    let chars: Vec<char> = component.chars().collect();
    let consumer = ComponentConsumer::new(&chars);

    self.compare_tok_chr(consumer, 0)
  }

  fn compare_tok_chr(&self, consumer: ComponentConsumer<'_>, tok_idx: usize) -> bool {
    let Some(token) = self.tokens.get(tok_idx) else {
      return consumer.is_empty();
    };

    match token {
      GlobToken::Literal(chr) => {
        let mut next = consumer;

        next.consume_literal(*chr) && self.compare_tok_chr(next, tok_idx + 1)
      }

      GlobToken::AnyChar => {
        let mut next = consumer;

        next.consume_anychar() && self.compare_tok_chr(next, tok_idx + 1)
      }

      GlobToken::AnySequence => {
        let mut next = consumer;

        loop {
          if self.compare_tok_chr(next, tok_idx + 1) {
            return true;
          }

          if !next.consume_anychar() {
            break;
          }
        }

        false
      }

      GlobToken::CharClass(items, false) => {
        let mut next = consumer;

        if let Some(chr) = next.remaining().first()
          && items.contains(chr)
        {
          next.consume_anychar();
          return self.compare_tok_chr(next, tok_idx + 1);
        }

        false
      }

      GlobToken::CharClass(items, true) => {
        let mut next = consumer;

        while let Some(chr) = next.remaining().first() {
          if !items.contains(chr) {
            break;
          }

          next.consume_anychar();

          if self.compare_tok_chr(next, tok_idx + 1) {
            return true;
          }
        }

        false
      }

      GlobToken::Alternatives(words) => {
        for word in words {
          let mut next = consumer;

          if next.consume_word(word) && self.compare_tok_chr(next, tok_idx + 1) {
            return true;
          }
        }

        false
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn assert_matches(pattern: &str, matching: &[&str], not_matching: &[&str]) {
    let glob = ComponentGlob::parse(pattern);

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
  fn literals_and_empty() {
    assert_matches("", &[""], &["a"]);
    assert_matches(
      "file.bin",
      &["file.bin"],
      &[
        "",
        "file.BIN",
        "file.bin.bak",
      ],
    );
    assert_matches("été", &["été"], &["ete", "étés"]);
  }

  #[test]
  fn any_char() {
    assert_matches("?", &["a", "é", "界"], &["", "ab"]);
    assert_matches("a?c", &["abc", "aéc"], &["ac", "abbc"]);
    assert_matches("??.bin", &["01.bin", "ab.bin"], &["1.bin", "123.bin"]);
  }

  #[test]
  fn any_sequence() {
    assert_matches("*", &["", "a", "abc"], &[]);
    assert_matches("a*", &["a", "ab", "abcd"], &["b", "ba"]);
    assert_matches("*z", &["z", "az", "abz"], &["za", "abc"]);
    assert_matches("a*b", &["ab", "axb", "axyb"], &["a", "ba", "axc"]);
    assert_matches(
      "a*b*c",
      &[
        "abc",
        "axbyc",
        "abbbc",
      ],
      &["ac", "ab", "axbyd"],
    );
  }

  #[test]
  fn backtracking() {
    assert_matches("*a", &["a", "ba", "aba"], &["ab"]);
    assert_matches("*ab", &["ab", "aab", "abab"], &["aba", "acb"]);
    assert_matches(
      "a*ab",
      &[
        "aab",
        "aaab",
        "abab",
      ],
      &["ab", "aac"],
    );
    assert_matches("*a*b", &["ab", "axab", "aaab"], &["ba", "ac"]);
    assert_matches("*?b", &["ab", "aab", "xxb"], &["b", "ba"]);
  }

  #[test]
  fn char_classes() {
    assert_matches("[0-9]", &["0", "5", "9"], &["", "a", "12"]);
    assert_matches(
      "file[0-2].bin",
      &[
        "file0.bin",
        "file2.bin",
      ],
      &[
        "file3.bin",
        "file12.bin",
      ],
    );
    assert_matches("[a-c]?", &["a1", "aé"], &["d1", "a", "a12"]);
  }

  #[test]
  fn repeated_char_classes() {
    assert_matches("[0-9]+", &["0", "123", "000"], &["", "abc", "12a"]);
    assert_matches(
      "file[0-9]+.bin",
      &[
        "file1.bin",
        "file123.bin",
      ],
      &[
        "file.bin",
        "file12a.bin",
      ],
    );
    assert_matches("[a-c]+z", &["az", "abcz", "ccz"], &["z", "adz", "ab"]);
  }

  #[test]
  fn alternatives() {
    assert_matches("{foo,bar}", &["foo", "bar"], &["", "baz", "foobar"]);
    assert_matches("{a,bc}z", &["az", "bcz"], &["bz", "abcz"]);
    assert_matches("{,foo}.bin", &[".bin", "foo.bin"], &["bar.bin"]);
    assert_matches("{a,ab}b", &["ab", "abb"], &["a", "abab"]);
  }

  #[test]
  fn combinations() {
    assert_matches(
      "{foo,bar}*.bin",
      &[
        "foo.bin",
        "bar123.bin",
        "foobar.bin",
      ],
      &["baz.bin", "foo.txt"],
    );
    assert_matches(
      "*[0-9]+.bin",
      &[
        "1.bin",
        "file123.bin",
      ],
      &[
        "file.bin",
        "file12a.bin",
      ],
    );
    assert_matches("{a,ab}*b", &["ab", "abb", "abxb"], &["a", "ac"]);
  }

  #[test]
  fn escaped_metacharacters() {
    assert_matches(r"\*", &["*"], &["", "abc"]);
    assert_matches(r"\?", &["?"], &["a", ""]);
    assert_matches(r"\[", &["["], &["a"]);
    assert_matches(r"\{", &["{"], &["a"]);
    assert_matches(r"a\*b", &["a*b"], &["ab", "axxb"]);
    assert_matches("abc\\", &["abc\\"], &["abc"]);
  }

  #[test]
  fn incomplete_constructs_are_literal() {
    assert_matches("[abc", &["[abc"], &["abc", "a"]);
    assert_matches("{foo,bar", &["{foo,bar"], &["foo", "bar"]);
  }
}
