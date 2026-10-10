use std::collections::BTreeMap;

use crate::virtual_path::VirtualPath;

/// A virtual path component, with everything the hierarchy nests below it.
///
/// Children are ordered by name so that a rendering is stable between runs, and
/// so that the branches of a level are laid out in the same order as the
/// lexicographic order the paths themselves are enumerated in.
struct Node {
  name: String,
  children: BTreeMap<String, Node>,
}

/// Renders virtual paths as a tree.
///
/// The hierarchy is built from the `/`-separated components of the paths given
/// and nothing else: whatever produced them, whether they are artifacts, cached
/// entries or overrides, never enters into it. The parents a path implies are
/// rendered as the branches leading to it, and are not paths in their own right.
///
/// `root` limits the rendering to a single branch, which is printed under its
/// full virtual path; every path is rendered when it is absent. A root naming
/// none of the paths and holding none below it renders as nothing at all, which
/// is what an empty result looks like everywhere else.
pub(crate) fn render(entries: &[&str], root: Option<&VirtualPath>) -> String {
  let Some(root) = root else {
    return render_roots(build(entries, "").values());
  };

  let name = root.to_string();
  let children = build(entries, &format!("{name}/"));

  if children.is_empty() && !entries.contains(&name.as_str()) {
    return String::new();
  }

  render_roots(std::iter::once(&Node { name, children }))
}

/// Builds the hierarchy of the virtual paths nested below `prefix`.
///
/// `prefix` is either empty, for the whole set, or a `/`-terminated virtual path.
/// Terminating it is what keeps a component from matching the start of a longer
/// one, so `game.sfc` never picks up entries under `game.sfcX`.
fn build(entries: &[&str], prefix: &str) -> BTreeMap<String, Node> {
  let mut nodes = BTreeMap::new();

  for entry in entries {
    let Some(relative) = entry.strip_prefix(prefix) else {
      continue;
    };
    if relative.is_empty() {
      continue;
    }

    let name = relative.split_once('/').map_or(relative, |(name, _)| name);

    nodes.entry(name.to_owned()).or_insert_with(|| Node {
      name: name.to_owned(),
      children: build(entries, &format!("{prefix}{name}/")),
    });
  }

  nodes
}

/// Renders `nodes` as the roots of the hierarchy, one per line, with each root
/// followed by its own subtree.
fn render_roots<'n>(nodes: impl ExactSizeIterator<Item = &'n Node>) -> String {
  let mut out = String::new();

  for node in nodes {
    out.push_str(&node.name);
    out.push('\n');
    render_children(node.children.values(), "", &mut out);
  }

  out
}

/// Renders `nodes` as the branches hanging off the line above them.
fn render_children<'n>(
  nodes: impl ExactSizeIterator<Item = &'n Node>,
  prefix: &str,
  out: &mut String,
) {
  let count = nodes.len();

  for (index, node) in nodes.enumerate() {
    let last = index + 1 == count;

    out.push_str(prefix);
    out.push_str(if last { "└── " } else { "├── " });
    out.push_str(&node.name);
    out.push('\n');

    let nested = format!("{prefix}{}", if last { "    " } else { "│   " });
    render_children(node.children.values(), &nested, out);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Paths for a project with two sources, listed out of order so that a rendering
  /// taken from them cannot be confused with one taken in the order the paths
  /// happen to be written down.
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

  #[test]
  fn renders_the_hierarchy_of_the_whole_set() {
    assert_eq!(render(&ENTRIES, None), HIERARCHY);
  }

  #[test]
  fn rendering_does_not_depend_on_the_order_paths_are_written_in() {
    let reversed = [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
      "game.sfc/tail.bin",
      "other.sfc/head.bin",
    ];

    assert_eq!(render(&reversed, None), HIERARCHY);
  }

  #[test]
  fn renders_only_the_requested_subtree() {
    assert_eq!(
      render(&ENTRIES, Some(&vpath("game.sfc"))),
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
      render(&ENTRIES, Some(&vpath("game.sfc/head.bin"))),
      "\
game.sfc/head.bin
└── TITLE.txt
"
    );
  }

  #[test]
  fn renders_a_path_nothing_is_nested_below() {
    assert_eq!(
      render(&ENTRIES, Some(&vpath("other.sfc/head.bin"))),
      "other.sfc/head.bin\n"
    );
  }

  #[test]
  fn renders_a_path_as_a_branch_when_it_is_both_a_path_and_a_parent() {
    // The set may hold both `game.sfc/head.bin` and a path below it, in which
    // case the root is a node rather than a leaf.
    let entries = [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
    ];

    assert_eq!(
      render(&entries, Some(&vpath("game.sfc/head.bin"))),
      "\
game.sfc/head.bin
└── TITLE.txt
"
    );
  }

  #[test]
  fn renders_nothing_for_an_empty_set() {
    assert_eq!(render(&[], None), "");
  }

  #[test]
  fn renders_nothing_for_a_root_naming_no_path_at_all() {
    for root in [
      // Never gathered, and nothing gathered below it.
      "game.sfc/missing.bin",
      // Held under another source.
      "third.sfc/head.bin",
      // A prefix of a path is not itself one.
      "game.sfc/head",
      "game.sfc/head.bin/TITLE",
      // Not a prefix of any path, although it shares one with a source.
      "game.sfcX",
    ] {
      assert_eq!(render(&ENTRIES, Some(&vpath(root))), "", "'{root}'");
    }

    assert_eq!(render(&[], Some(&vpath("game.sfc"))), "");
  }

  #[test]
  fn keeps_deeper_levels_of_one_branch_from_mixing_with_another() {
    // `game.sfc` and `game.sfcX` share every character but the last one.
    let entries = [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
      "game.sfcX/head.bin",
    ];

    assert_eq!(
      render(&entries, Some(&vpath("game.sfc"))),
      "\
game.sfc
└── head.bin
    └── TITLE.txt
"
    );
  }
}
