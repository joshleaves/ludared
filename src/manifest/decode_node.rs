use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct CodecNode {
  pub id: String,
  pub version: u32,
  #[serde(default)]
  pub args: serde_json::Value,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct DecodeNode {
  pub name: String,
  pub codec: CodecNode,
  pub outputs: Vec<String>,

  /// The outputs of this decode node that are materialized in the workspace.
  ///
  /// The declaration is local to the decode node that produced the artifact, so
  /// each entry is a name relative to that node and matches one of `outputs`
  /// exactly, including names carrying `/`-separated components.
  ///
  /// A node declaring none leaves the field out entirely, so a decode node is
  /// written back exactly as it was read when no override was ever added to it.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub overrides: Vec<String>,

  #[serde(default)]
  pub decodes: HashMap<String, Vec<DecodeNode>>,
}

impl DecodeNode {
  /// Returns whether this decode node declares `output` as an override.
  ///
  /// An override is declared by the decode node that produced the artifact, so
  /// the question only ever concerns an output of this node.
  pub(crate) fn has_override_for(&self, output: &str) -> bool {
    self.overrides.iter().any(|name| name == output)
  }

  /// Returns whether `output` is a leaf of the decode tree.
  ///
  /// An empty decode bucket still counts as terminal: `decode add` creates one
  /// for the artifact it is pointed at, and none of those decodes produce
  /// anything.
  pub(crate) fn is_terminal_output(&self, output: &str) -> bool {
    self
      .decodes
      .get(output)
      .is_none_or(|decodes| decodes.is_empty())
  }

  /// Resolves or creates the mutable decode bucket for an artifact relative to
  /// this decode node.
  ///
  /// The target artifact itself must already exist in this node's `outputs`.
  /// This method only creates the `Vec<DecodeNode>` associated with that
  /// artifact; it never creates an artifact or an output path.
  ///
  /// When `remaining` extends beyond a matching output, the method recursively
  /// searches the decode nodes already attached to that output. Intermediate
  /// decode nodes are never created implicitly.
  ///
  /// # Returns
  ///
  /// - `Some(decodes)` when `remaining` resolves to an existing artifact. Its
  ///   decode bucket is created if it does not already exist.
  /// - `None` when no existing output and decode branch can resolve `remaining`.
  ///
  /// This distinction ensures that decode buckets may be created lazily without
  /// allowing arbitrary virtual paths to create artifacts that were never
  /// produced by a decoder.
  pub(crate) fn get_or_create_decodes_mut(
    &mut self,
    remaining: &[&str],
  ) -> Option<&mut Vec<DecodeNode>> {
    for output in &self.outputs {
      let output_components: Vec<&str> = output.split('/').collect();

      if !remaining.starts_with(&output_components) {
        continue;
      }

      let remaining = &remaining[output_components.len()..];

      if remaining.is_empty() {
        return Some(self.decodes.entry(output.clone()).or_default());
      }

      let decodes = self.decodes.get_mut(output)?;

      for decode in decodes {
        if let Some(decodes) = decode.get_or_create_decodes_mut(remaining) {
          return Some(decodes);
        }
      }

      return None;
    }

    None
  }

  /// Resolves the decode node that produced an artifact for a path relative to
  /// this decode node.
  ///
  /// An output name can hold several `/`-separated components, and two outputs of
  /// the same node may share a prefix: `data` and `data/head.bin` can both be
  /// produced here, which leaves `data/head.bin` with two readings. An output
  /// matching the path outright wins over one it merely continues past, so the
  /// path resolves to the artifact this node produced rather than to whatever
  /// hangs below the output it starts with. That holds however the outputs happen
  /// to be declared.
  ///
  /// # Returns
  ///
  /// - `Some((node, output))` when `remaining` resolves to an output of this
  ///   node, `output` being the name of that output and `node` the decode node
  ///   producing it.
  /// - `None` when no output in this branch can resolve `remaining`.
  pub(crate) fn resolve_output<'s>(
    &'s self,
    remaining: &[&str],
  ) -> Option<(&'s DecodeNode, &'s str)> {
    let output = self.producing_output(remaining)?;

    if components_of(output).as_slice() == remaining {
      return Some((self, output));
    }

    // An output nothing decodes is a leaf, so nothing sits below it.
    let decodes = self.decodes.get(output)?;

    let nested = &remaining[components_of(output).len()..];

    for decode in decodes {
      if let Some(producer) = decode.resolve_output(nested) {
        return Some(producer);
      }
    }

    None
  }

  /// Resolves the decode node that produced an artifact for a path relative to
  /// this decode node, for callers holding a unique borrow of that node.
  ///
  /// This mirrors [`Self::resolve_output`] in every respect, down to the branch
  /// both of them settle on, handing the output name back by value so that the
  /// decode node stays exclusively borrowed and can be amended afterwards. Like
  /// [`Self::get_or_create_decodes_mut`], it never creates a decode bucket.
  ///
  /// # Returns
  ///
  /// - `Some((node, output))` when `remaining` resolves to an output of this
  ///   node, `output` being the name of that output and `node` the decode node
  ///   producing it.
  /// - `None` when no output in this branch can resolve `remaining`.
  pub(crate) fn resolve_output_mut(
    &mut self,
    remaining: &[&str],
  ) -> Option<(&mut DecodeNode, String)> {
    let output = self.producing_output(remaining)?.to_owned();

    if components_of(&output).as_slice() == remaining {
      return Some((self, output));
    }

    // An output nothing decodes is a leaf, so nothing sits below it.
    let decodes = self.decodes.get_mut(&output)?;

    let nested = &remaining[components_of(&output).len()..];

    for decode in decodes.iter_mut() {
      if let Some(producer) = decode.resolve_output_mut(nested) {
        return Some(producer);
      }
    }

    None
  }

  /// Returns the output of this node that a path resolves to, or through when it
  /// carries on below one.
  ///
  /// No two outputs sharing an input artifact stand in an ancestor relation with
  /// one another, so at most one of them can be a prefix of any given path: were
  /// two of them to be, each would be a prefix of the other. The first output the
  /// path runs into is therefore the only one it can resolve through, and the
  /// order the outputs were declared in never decides anything.
  fn producing_output(&self, remaining: &[&str]) -> Option<&str> {
    self
      .outputs
      .iter()
      .find(|output| remaining.starts_with(&components_of(output)))
      .map(String::as_str)
  }
}

/// The `/`-separated components of an output name.
fn components_of(output: &str) -> Vec<&str> {
  output.split('/').collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A decode node producing `outputs`, optionally with `overrides` declared.
  fn node(name: &str, outputs: &[&str], overrides: &[&str]) -> DecodeNode {
    DecodeNode {
      name: name.to_owned(),
      codec: CodecNode::default(),
      outputs: outputs.iter().map(|output| (*output).to_owned()).collect(),
      overrides: overrides
        .iter()
        .map(|output| (*output).to_owned())
        .collect(),
      decodes: HashMap::new(),
    }
  }

  /// Attaches `nested` under `output`, as a decode of that artifact.
  fn with_decode(mut node: DecodeNode, output: &str, nested: Vec<DecodeNode>) -> DecodeNode {
    node.decodes.insert(output.to_owned(), nested);
    node
  }

  /// The components of a virtual path, as the resolvers receive them.
  fn components(path: &str) -> Vec<&str> {
    path.split('/').collect()
  }

  /// A tree nesting decodes two levels deep below a source, where the first
  /// artifact is named by a path of its own.
  ///
  /// ```text
  /// bank.bin          -> override declared on the outer node
  /// bank.bin/pal.bin  -> produced by a nested node, whose output is `pal.bin`
  /// bank.bin/pal.bin/entry.txt -> one level deeper still
  /// ```
  fn nested_tree() -> DecodeNode {
    let palette = with_decode(
      node("palette", &["pal.bin"], &["pal.bin"]),
      "pal.bin",
      vec![with_decode(
        node("entry", &["entry.txt"], &["entry.txt"]),
        "entry.txt",
        vec![],
      )],
    );

    let tail = with_decode(node("inner", &["tail.bin"], &[]), "tail.bin", vec![]);

    with_decode(
      node(
        "bank",
        &[
          "head.bin",
          "bank.bin",
        ],
        &[],
      ),
      "bank.bin",
      // The nested decodes are tried in order, so the tree covers both a decode
      // that resolves and one that does not.
      vec![tail, palette],
    )
  }

  #[test]
  fn resolves_an_output_of_the_node_itself() {
    let tree = nested_tree();

    let (producer, output) = tree.resolve_output(&components("bank.bin")).unwrap();

    assert_eq!(output, "bank.bin");
    assert_eq!(producer.name, "bank");
  }

  #[test]
  fn resolves_an_output_produced_by_a_nested_decode() {
    let tree = nested_tree();

    let (producer, output) = tree
      .resolve_output(&components("bank.bin/pal.bin"))
      .unwrap();

    // The producer is the node that made the artifact, not the one that made its
    // parent.
    assert_eq!(output, "pal.bin");
    assert_eq!(producer.name, "palette");
  }

  #[test]
  fn resolves_through_several_levels_of_nesting() {
    let tree = nested_tree();

    let (producer, output) = tree
      .resolve_output(&components("bank.bin/pal.bin/entry.txt"))
      .unwrap();

    assert_eq!(output, "entry.txt");
    assert_eq!(producer.name, "entry");
  }

  #[test]
  fn consumes_an_output_name_carrying_several_components() {
    // An output name is a single artifact, however many components it has, and
    // the whole of it is consumed before the search descends any further.
    let tree = node("nested", &["data/gfx/head.bin"], &["data/gfx/head.bin"]);

    let (producer, output) = tree
      .resolve_output(&components("data/gfx/head.bin"))
      .unwrap();

    assert_eq!(output, "data/gfx/head.bin");
    assert_eq!(producer.name, "nested");

    // A decode hanging off it is still reached, one component at a time.
    let tree = with_decode(
      tree,
      "data/gfx/head.bin",
      vec![node(
        "deep",
        &["tail.bin"],
        &[],
      )],
    );
    let (producer, output) = tree
      .resolve_output(&components("data/gfx/head.bin/tail.bin"))
      .unwrap();

    assert_eq!(output, "tail.bin");
    assert_eq!(producer.name, "deep");
  }

  #[test]
  fn the_two_resolvers_resolve_an_ambiguous_tree_identically() {
    // Every resolution below has a second reading available somewhere in the
    // tree, so taking the first one that happened to match would show up here.
    let ambiguous = || {
      let tree = with_decode(
        with_decode(
          with_decode(
            node(
              "split",
              &[
                "data",
                "data/head.bin",
                "a",
              ],
              &[],
            ),
            "data",
            vec![node(
              "nested",
              &[
                "head.bin",
                "elsewhere.bin",
              ],
              &[],
            )],
          ),
          "data/head.bin",
          vec![node(
            "deep",
            &["leaf.bin"],
            &[],
          )],
        ),
        "a",
        vec![node(
          "under_a",
          &["b/c"],
          &[],
        )],
      );

      with_decode(
        tree,
        "a/b",
        vec![node(
          "deeper",
          &["c"],
          &[],
        )],
      )
    };

    for path in [
      "data",
      "data/head.bin",
      "data/elsewhere.bin",
      "data/head.bin/leaf.bin",
      "a",
      "a/b/c",
      "missing.bin",
      "data/missing.bin",
    ] {
      let tree = ambiguous();
      let mut tree_mut = ambiguous();

      let expected = tree.resolve_output(&components(path));
      let actual = tree_mut.resolve_output_mut(&components(path));

      match (expected, actual) {
        (Some((node, output)), Some((mut_node, mut_output))) => {
          assert_eq!(
            node.name, mut_node.name,
            "'{path}' resolved to another node"
          );
          assert_eq!(output, mut_output, "'{path}' resolved to another output");
        }
        (None, None) => {}
        (expected, actual) => panic!("'{path}' resolved to {expected:?} but {actual:?}"),
      }
    }
  }

  #[test]
  fn resolves_nothing_for_a_path_beyond_the_output_name() {
    let tree = node("bank", &["head.bin"], &[]);

    // The output was consumed in full, so what follows is not a continuation of
    // the artifact but an unknown path.
    assert!(
      tree
        .resolve_output(&components("head.bin/tail.bin"))
        .is_none()
    );
    assert!(tree.resolve_output(&components("head")).is_none());
    assert!(tree.resolve_output(&components("missing.bin")).is_none());
    assert!(tree.resolve_output(&[]).is_none());
  }

  #[test]
  fn resolves_nothing_below_an_output_with_no_decode() {
    let tree = node("bank", &["head.bin"], &[]);

    // An artifact nobody decodes is a leaf, whatever else the tree holds.
    assert!(tree.resolve_output(&components("missing.bin")).is_none());
  }

  #[test]
  fn resolves_a_prefix_shared_by_several_outputs_to_one_of_them() {
    // `head.bin` and `head.bin.cmp` share a prefix, but only the first matches.
    let tree = node(
      "bank",
      &[
        "head.bin",
        "head.bin.cmp",
      ],
      &[],
    );

    assert_eq!(
      tree.resolve_output(&components("head.bin")).unwrap().1,
      "head.bin"
    );
  }

  #[test]
  fn the_mutable_resolver_agrees_with_the_shared_one_and_hands_back_a_usable_name() {
    // The mutable resolver is the same search, so it settles on the same branch
    // and hands back the same output name; what it adds is a node the caller may
    // amend.
    let mut tree = nested_tree();

    for path in [
      "bank.bin",
      "bank.bin/tail.bin",
      "bank.bin/pal.bin",
      "bank.bin/pal.bin/entry.txt",
    ] {
      let expected = nested_tree()
        .resolve_output(&components(path))
        .map(|(node, output)| (node.name.clone(), output.to_owned()));
      let actual = tree
        .resolve_output_mut(&components(path))
        .map(|(node, output)| (node.name.clone(), output));

      match (expected, actual) {
        (Some(expected), Some(actual)) => {
          assert_eq!(actual, expected, "'{path}' resolved elsewhere");
          // The name belongs to this node, which the caller may now amend.
          let (node, output) = tree.resolve_output_mut(&components(path)).unwrap();
          assert!(node.outputs.contains(&output));
        }
        (None, None) => {}
        (expected, actual) => panic!("'{path}' resolved to {expected:?} but {actual:?}"),
      }
    }

    // Paths nothing can resolve stay unresolvable through the mutable borrow too.
    for path in [
      "bank.bin/pal.bin/entry",
      "missing.bin",
      "bank.bin/pal.bin/entry.txt/deeper",
    ] {
      assert!(
        tree.resolve_output_mut(&components(path)).is_none(),
        "'{path}'"
      );
    }
  }
}
