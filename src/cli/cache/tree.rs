use std::collections::BTreeMap;

use log::*;

use clap::Args;
use clap_complete::engine::ArgValueCompleter;

use crate::cli::completions::virtual_path::complete_virtual_path;
use crate::errors::app_error::AppError;
use crate::project::Project;
use crate::project::cache::errors::CacheError;
use crate::virtual_path::VirtualPath;

#[derive(Args)]
pub(crate) struct CacheTreeArgs {
  /// Virtual path to render as the root of the tree
  #[arg(value_name = "PATH", add = ArgValueCompleter::new(complete_virtual_path))]
  virtual_path: Option<String>,
}

pub(crate) fn command_cache_tree(project: &Project, args: &CacheTreeArgs) -> Result<(), AppError> {
  let root = args
    .virtual_path
    .as_deref()
    .map(VirtualPath::new)
    .transpose()?;
  match root.as_ref() {
    Some(root) => info!("Rendering '{root}' as a tree from the decode cache"),
    None => info!("Rendering the decode cache as a tree"),
  }

  let entries = project.cache.entries().collect::<Vec<_>>();

  print!("{}", render(&entries, root.as_ref())?);

  Ok(())
}

/// A virtual path component, with everything the index nests below it.
///
/// Children are ordered by name so that a rendering is stable between runs, and
/// so that the branches of a level are laid out in the same order as the
/// lexicographic order the index itself stores them in.
struct Node {
  name: String,
  children: BTreeMap<String, Node>,
}

/// Renders the virtual paths held in the decode cache as a tree.
///
/// The hierarchy is built from the `/`-separated components of the indexed
/// virtual paths and nothing else: the manifest, the workspace, the sources and
/// the blobs are never consulted, and no tree is retained past this call. The
/// index remains the flat source of truth this view is derived from.
///
/// `root` limits the rendering to a single branch of the index, which is printed
/// under its full virtual path; the whole index is rendered when it is absent.
///
/// # Errors
///
/// Returns [`CacheError::NotIndexed`] if `root` names neither an indexed virtual
/// path nor a parent of one. An index holding nothing renders as an empty string
/// and is not an error.
pub(crate) fn render(entries: &[&str], root: Option<&VirtualPath>) -> Result<String, CacheError> {
  let Some(root) = root else {
    let index = build(entries, "");
    return Ok(render_roots(index.values()));
  };

  let name = root.to_string();
  let children = build(entries, &format!("{name}/"));

  if !entries.contains(&name.as_str()) && children.is_empty() {
    return Err(CacheError::NotIndexed(name));
  }

  let root = Node { name, children };

  Ok(render_roots(std::iter::once(&root)))
}

/// Builds the hierarchy of the virtual paths nested below `prefix`.
///
/// `prefix` is either empty, for the whole index, or a `/`-terminated virtual
/// path. Terminating it is what keeps a component from matching the start of a
/// longer one, so `game.sfc` never picks up entries under `game.sfcX`.
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

  /// Index entries for a project with two sources, listed out of order so that a
  /// rendering taken from the index cannot be confused with one taken in the
  /// order the entries happen to be written down.
  const ENTRIES: [&str; 4] = [
    "other.sfc/head.bin",
    "game.sfc/tail.bin",
    "game.sfc/head.bin/TITLE.txt",
    "game.sfc/head.bin",
  ];

  const INDEX: &str = "\
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
  fn renders_the_hierarchy_of_the_whole_index() {
    assert_eq!(render(&ENTRIES, None).unwrap(), INDEX);
  }

  #[test]
  fn rendering_does_not_depend_on_the_order_entries_are_written_in() {
    let reversed = [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
      "game.sfc/tail.bin",
      "other.sfc/head.bin",
    ];

    assert_eq!(render(&reversed, None).unwrap(), INDEX);
  }

  #[test]
  fn renders_only_the_requested_subtree() {
    assert_eq!(
      render(&ENTRIES, Some(&vpath("game.sfc"))).unwrap(),
      "\
game.sfc
├── head.bin
│   └── TITLE.txt
└── tail.bin
"
    );
  }

  #[test]
  fn renders_a_subtree_rooted_at_an_indexed_path() {
    assert_eq!(
      render(&ENTRIES, Some(&vpath("game.sfc/head.bin"))).unwrap(),
      "\
game.sfc/head.bin
└── TITLE.txt
"
    );
  }

  #[test]
  fn renders_a_subtree_rooted_at_an_indexed_path_nothing_is_nested_below() {
    assert_eq!(
      render(&ENTRIES, Some(&vpath("other.sfc/head.bin"))).unwrap(),
      "other.sfc/head.bin\n"
    );
  }

  #[test]
  fn renders_an_indexed_path_as_a_branch_when_it_is_both_a_path_and_a_parent() {
    // The index may hold both `game.sfc/head.bin` and a path below it, in which
    // case the root is a node rather than a leaf.
    let entries = [
      "game.sfc/head.bin",
      "game.sfc/head.bin/TITLE.txt",
    ];

    assert_eq!(
      render(&entries, Some(&vpath("game.sfc/head.bin"))).unwrap(),
      "\
game.sfc/head.bin
└── TITLE.txt
"
    );
  }

  #[test]
  fn renders_nothing_for_an_empty_index() {
    assert_eq!(render(&[], None).unwrap(), "");
  }

  #[test]
  fn fails_when_the_root_names_neither_an_indexed_path_nor_a_parent_of_one() {
    for root in [
      // Never indexed, and nothing indexed below it.
      "game.sfc/missing.bin",
      // Indexed under another source.
      "third.sfc/head.bin",
      // A prefix of an indexed path is not itself indexed.
      "game.sfc/head",
      "game.sfc/head.bin/TITLE",
      // Not a prefix of any indexed path, although it shares one with a source.
      "game.sfcX",
    ] {
      let CacheError::NotIndexed(path) = render(&ENTRIES, Some(&vpath(root))).unwrap_err();
      assert_eq!(path, root);
    }
  }

  #[test]
  fn fails_when_the_root_is_given_for_an_empty_index() {
    let CacheError::NotIndexed(path) = render(&[], Some(&vpath("game.sfc"))).unwrap_err();
    assert_eq!(path, "game.sfc");
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
      render(&entries, Some(&vpath("game.sfc"))).unwrap(),
      "\
game.sfc
└── head.bin
    └── TITLE.txt
"
    );
  }
}
