#[derive(Clone, Copy)]
pub(super) struct ComponentConsumer<'a> {
  chars: &'a [char],
  idx: usize,
}

impl<'a> ComponentConsumer<'a> {
  pub(super) fn new(chars: &'a [char]) -> Self {
    Self { chars, idx: 0 }
  }

  pub(super) fn remaining(&self) -> &[char] {
    &self.chars[self.idx..]
  }

  pub(super) fn is_empty(&self) -> bool {
    self.idx == self.chars.len()
  }

  pub(super) fn consume(&mut self, chars: &[char]) -> bool {
    if !self.remaining().starts_with(chars) {
      return false;
    }

    self.idx += chars.len();
    true
  }

  pub(super) fn consume_literal(&mut self, chr: char) -> bool {
    self.consume(&[chr])
  }

  pub(super) fn consume_anychar(&mut self) -> bool {
    if self.is_empty() {
      return false;
    }

    self.idx += 1;
    true
  }

  pub(super) fn consume_word(&mut self, word: &str) -> bool {
    let mut remaining = self.remaining().iter().copied();
    let mut count = 0;

    for ch in word.chars() {
      if remaining.next() != Some(ch) {
        return false;
      }
      count += 1;
    }

    self.idx += count;
    true
  }
}
