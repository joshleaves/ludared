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
  #[serde(default)]
  pub decodes: HashMap<String, Vec<DecodeNode>>,
}

impl DecodeNode {
  /// Resolves decode nodes for a path relative to this decode node.
  ///
  /// Each output is treated as a single logical artifact, even when its name
  /// contains multiple `/`-separated path components. The method matches the
  /// beginning of `remaining` against each output, consumes the full matching
  /// output path, and recursively continues through the decode nodes attached
  /// to that artifact.
  ///
  /// # Returns
  ///
  /// - `Some(decodes)` when `remaining` resolves to an existing artifact that
  ///   has one or more decode nodes attached.
  /// - `Some(&[])` when `remaining` resolves to an existing artifact that has
  ///   no decode nodes attached.
  /// - `None` when no output in this branch can resolve `remaining`.
  ///
  /// `None` is not treated as an error here because callers may need to try
  /// other sibling decode nodes before deciding that the virtual path cannot
  /// be resolved.
  pub(crate) fn get_decodes(&self, remaining: &[&str]) -> Option<&[DecodeNode]> {
    for output in &self.outputs {
      let output_components: Vec<&str> = output.split('/').collect();

      if !remaining.starts_with(&output_components) {
        continue;
      }

      let remaining = &remaining[output_components.len()..];

      let decodes = self.decodes.get(output).map(Vec::as_slice).unwrap_or(&[]);

      if remaining.is_empty() {
        return Some(decodes);
      }

      for decode in decodes {
        if let Some(decodes) = decode.get_decodes(remaining) {
          return Some(decodes);
        }
      }
    }

    None
  }

  /// Resolves mutable decode nodes for a path relative to this decode node.
  ///
  /// Each output is treated as a single logical artifact, even when its name
  /// contains multiple `/`-separated path components. The method matches the
  /// beginning of `remaining` against each output, consumes the full matching
  /// output path, and recursively continues through the decode nodes attached
  /// to that artifact.
  ///
  /// Unlike [`Self::get_or_create_decodes_mut`], this method never creates a
  /// decode bucket.
  ///
  /// # Returns
  ///
  /// - `Some(decodes)` when `remaining` resolves to an existing artifact that
  ///   has one or more decode nodes attached.
  /// - `Some(&mut [])` when `remaining` resolves to an existing artifact that
  ///   has an existing but empty decode bucket.
  /// - `None` when no output in this branch can resolve `remaining`, or when the
  ///   target artifact has no decode bucket.
  ///
  /// `None` is not treated as an error here because callers may need to try
  /// other sibling decode nodes before deciding that the virtual path cannot
  /// be resolved.
  pub(crate) fn get_decodes_mut(&mut self, remaining: &[&str]) -> Option<&mut [DecodeNode]> {
    for output in &self.outputs {
      let output_components: Vec<&str> = output.split('/').collect();

      if !remaining.starts_with(&output_components) {
        continue;
      }

      let remaining = &remaining[output_components.len()..];

      let decodes = self.decodes.get_mut(output)?;

      if remaining.is_empty() {
        return Some(decodes.as_mut_slice());
      }

      for decode in decodes {
        if let Some(decodes) = decode.get_decodes_mut(remaining) {
          return Some(decodes);
        }
      }

      return None;
    }

    None
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
}
