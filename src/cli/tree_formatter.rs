use std::collections::BTreeMap;

use ludared::virtual_path::VirtualPath;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TreeNodeKind {
  Decode,
  Path,
}

pub(crate) type Tree = BTreeMap<(TreeNodeKind, String), TreeNode>;

pub(crate) struct TreeNode {
  name: String,
  pub(crate) children: Tree,
}

impl TreeNode {
  pub(crate) fn new(name: impl Into<String>) -> Self {
    Self {
      name: name.into(),
      children: Tree::new(),
    }
  }

  pub(crate) fn child(&mut self, name: impl Into<String>) -> &mut Self {
    self.child_with_kind(name.into(), TreeNodeKind::Path)
  }

  pub(crate) fn decode_child(&mut self, name: impl Into<String>) -> &mut Self {
    self.child_with_kind(name.into(), TreeNodeKind::Decode)
  }

  fn child_with_kind(&mut self, name: String, kind: TreeNodeKind) -> &mut Self {
    let key = (kind, name.clone());
    self.children.entry(key).or_insert_with(|| Self::new(name))
  }
}

pub(crate) fn insert_path<'tree>(tree: &'tree mut Tree, path: &str) -> &'tree mut TreeNode {
  let mut components = path.split('/');
  let name = components.next().unwrap_or_default().to_owned();
  let mut node = tree
    .entry((TreeNodeKind::Path, name.clone()))
    .or_insert_with(|| TreeNode::new(name));

  for component in components {
    node = node.child(component);
  }

  node
}

/// Builds a tree from virtual paths, optionally rooted at a path.
pub(crate) fn build_path_tree(entries: &[&str], root: Option<&VirtualPath>) -> Tree {
  let Some(root) = root else {
    let mut tree = Tree::new();
    for entry in entries {
      insert_path(&mut tree, entry);
    }
    return tree;
  };

  let name = root.to_string();
  let prefix = format!("{name}/");
  let mut root_node = TreeNode::new(name.clone());
  let mut found = entries.contains(&name.as_str());

  for entry in entries {
    if let Some(relative) = entry.strip_prefix(&prefix) {
      found = true;
      insert_path(&mut root_node.children, relative);
    }
  }

  if !found {
    return Tree::new();
  }

  BTreeMap::from([((TreeNodeKind::Path, name), root_node)])
}

/// Renders an already-built tree with stable, lexicographic child ordering.
pub(crate) fn render(tree: &Tree) -> String {
  let mut out = String::new();

  for node in tree.values() {
    out.push_str(&node.name);
    out.push('\n');
    render_children(&node.children, "", &mut out);
  }

  out
}

fn render_children(nodes: &Tree, prefix: &str, out: &mut String) {
  let count = nodes.len();

  for (index, node) in nodes.values().enumerate() {
    let last = index + 1 == count;

    out.push_str(prefix);
    out.push_str(if last { "└── " } else { "├── " });
    out.push_str(&node.name);
    out.push('\n');

    let nested = format!("{prefix}{}", if last { "    " } else { "│   " });
    render_children(&node.children, &nested, out);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const ENTRIES: [&str; 4] = [
    "other.sfc/head.bin",
    "game.sfc/tail.bin",
    "game.sfc/head.bin/TITLE.txt",
    "game.sfc/head.bin",
  ];

  const HIERARCHY: &str = "\
game.sfc
├── head.bin
│   └── TITLE.txt
└── tail.bin
other.sfc
└── head.bin
";

  fn vpath(path: &str) -> VirtualPath {
    VirtualPath::new(path).unwrap()
  }

  fn render_paths(entries: &[&str], root: Option<&VirtualPath>) -> String {
    render(&build_path_tree(entries, root))
  }

  #[test]
  fn renders_the_hierarchy_of_the_whole_set() {
    assert_eq!(render_paths(&ENTRIES, None), HIERARCHY);
  }

  #[test]
  fn rendering_does_not_depend_on_the_order_paths_are_written_in() {
    let reversed = [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
      "game.sfc/tail.bin",
      "other.sfc/head.bin",
    ];

    assert_eq!(render_paths(&reversed, None), HIERARCHY);
  }

  #[test]
  fn renders_only_the_requested_subtree() {
    assert_eq!(
      render_paths(&ENTRIES, Some(&vpath("game.sfc"))),
      "\
game.sfc
├── head.bin
│   └── TITLE.txt
└── tail.bin
"
    );
  }

  #[test]
  fn renders_a_subtree_rooted_at_a_path() {
    assert_eq!(
      render_paths(&ENTRIES, Some(&vpath("game.sfc/head.bin"))),
      "\
game.sfc/head.bin
└── TITLE.txt
"
    );
  }

  #[test]
  fn renders_a_path_nothing_is_nested_below() {
    assert_eq!(
      render_paths(&ENTRIES, Some(&vpath("other.sfc/head.bin"))),
      "other.sfc/head.bin\n"
    );
  }

  #[test]
  fn renders_a_path_as_a_branch_when_it_is_both_a_path_and_a_parent() {
    let entries = [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
    ];

    assert_eq!(
      render_paths(&entries, Some(&vpath("game.sfc/head.bin"))),
      "\
game.sfc/head.bin
└── TITLE.txt
"
    );
  }

  #[test]
  fn renders_nothing_for_an_empty_set() {
    assert_eq!(render_paths(&[], None), "");
  }

  #[test]
  fn renders_nothing_for_a_root_naming_no_path_at_all() {
    for root in [
      "game.sfc/missing.bin",
      "third.sfc/head.bin",
      "game.sfc/head",
      "game.sfc/head.bin/TITLE",
      "game.sfcX",
    ] {
      assert_eq!(render_paths(&ENTRIES, Some(&vpath(root))), "", "'{root}'");
    }

    assert_eq!(render_paths(&[], Some(&vpath("game.sfc"))), "");
  }

  #[test]
  fn keeps_deeper_levels_of_one_branch_from_mixing_with_another() {
    let entries = [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
      "game.sfcX/head.bin",
    ];

    assert_eq!(
      render_paths(&entries, Some(&vpath("game.sfc"))),
      "\
game.sfc
└── head.bin
    └── TITLE.txt
"
    );
  }
}
