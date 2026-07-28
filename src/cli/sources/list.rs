use crate::formatting::format_bytes;
use crate::formatting::format_size;
use crate::{app_error::AppError, project::Project, source::Source};
use clap::Args;
use log::*;
use std::path::Path;

#[derive(Args)]
pub(crate) struct SourcesListArgs {
  /// Show whether source files are present
  #[arg(long)]
  status: bool,
}

pub(crate) fn command_sources_list(args: &SourcesListArgs) -> Result<(), AppError> {
  let project = Project::load_default()?;
  info!("Listing sources for project {}", project.name());
  if project.manifest.sources.is_empty() {
    warn!("No source files defined in project");
    return Ok(());
  }
  for (source_name, source) in project.manifest.sources.iter() {
    display_source_entry(&project, source_name, source, args);
  }

  Ok(())
}

fn display_source_entry(
  project: &Project,
  source_name: &Path,
  source: &Source,
  args: &SourcesListArgs,
) {
  println!("{}", source_name.display());
  if args.status {
    let source_path = project.source_path(source_name);
    if source_path.is_file() {
      println!("  Status: ✓ Present");
    } else {
      println!("  Status: ✗ Missing");
    }
  }
  if let Some(source_label) = &source.label {
    println!("  Label:  {}", source_label);
  }
  if let Some(source_size) = &source.size {
    println!(
      "  Size:   {} ({} bytes)",
      format_size(*source_size),
      format_bytes(*source_size)
    );
  }
  println!("  SHA256: {}", &source.sha256);
}
