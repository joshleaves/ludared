use crate::cli::tree_formatter::render;
use crate::virtual_path::VirtualPath;

/// Prints `entries`, optionally scoped to `root`, as a flat listing or as a tree.
///
/// The listing commands differ in what they enumerate and in nothing else, so
/// what is done with the entries once gathered is the same for all of them.
///
/// The two renderings scope a root differently, and both are kept as they were:
/// a flat listing filters the entries down to the ones within `root` before
/// printing them, while the renderer is handed everything and scopes the tree
/// itself, which is what lets it draw the implicit parents leading to an entry.
pub(crate) fn print(entries: &[VirtualPath], root: Option<&VirtualPath>, tree: bool) {
  if tree {
    let rendered = entries
      .iter()
      .map(VirtualPath::to_string)
      .collect::<Vec<_>>();
    let rendered = rendered.iter().map(String::as_str).collect::<Vec<_>>();

    print!("{}", render(&rendered, root));

    return;
  }

  for entry in entries {
    if root.is_none_or(|root| entry.is_within(root)) {
      println!("{entry}");
    }
  }
}
