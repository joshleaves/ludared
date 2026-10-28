use crate::cli::codecs::registry;
use crate::cli::tree_formatter;
use clap::Args;
use ludared::errors::app_error::AppError;

#[derive(Args)]
pub(crate) struct CodecsListArgs {
  /// Display the codec ids as a tree
  ///
  /// The `/`-separated components of an id are read as a hierarchy, so the ids
  /// nest under the prefixes they share.
  #[arg(long)]
  tree: bool,
}

pub(crate) fn command_codecs_list(args: &CodecsListArgs) -> Result<(), AppError> {
  let registry = registry()?;
  let mut codecs = registry.all_codecs().collect::<Vec<_>>();

  // The registry yields built-in codecs ahead of script codecs, which says
  // nothing about how they are related, so the listing is ordered by id instead.
  codecs.sort_by(|a, b| a.id().cmp(b.id()));

  if args.tree {
    let ids = codecs.iter().map(|codec| codec.id()).collect::<Vec<_>>();

    print!("{}", tree_formatter::render(&ids, None));

    return Ok(());
  }

  for codec in codecs {
    println!("{}: {}", codec.id(), codec.name());
  }

  Ok(())
}
