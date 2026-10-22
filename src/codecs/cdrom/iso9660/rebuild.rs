//! Rebuilding an ISO 9660 image from the artifacts a decode produced.
//!
//! A rebuild follows the layout the metadata records: every file goes back where
//! it was, every directory keeps the records it held and the bytes they filled,
//! and the System Area and the bytes outside the filesystem go back where they
//! lay. What it has to decide is what to do with the files whose contents have
//! changed, which the layout cannot answer on its own.
//!
//! A file whose contents shrank, or grew within the blocks it already had, keeps
//! its extent: nothing else in the image has to move for it. One that no longer
//! fits there is given room — preferably the blocks right after its own, so that
//! it stays where it was, and failing that the first free room in the image, or
//! room after the image's end. Only the first two of those keep the file at its
//! extent, which a game that seeks to a block of its own relies on, so keeping
//! the extent is preferred over keeping the image's length, and a move is logged.
//!
//! An image may grow, and one holding a file whose data lies beyond it — a
//! mixed-mode disc keeps a file on the track that follows the image's — does not
//! change that. The logical blocks of a rebuild are the track's sectors in order,
//! so a block number in a record is an address in one space with the image's own
//! length, and the blocks past that length sit where the bytes after the track
//! do. Growing the image by N blocks therefore moves everything at or past its
//! old end N blocks later: the records that describe data beyond the image move
//! with it, keeping their distance from the image's end, and so does the Volume
//! Space Size where it declared blocks past that end. A record whose extent
//! straddles the image's end describes one run of blocks across it, which no
//! shift can keep whole — the growth would insert blocks into the middle of it —
//! so an image holding one refuses to grow.
//!
//! Two more things are refused rather than done silently, because the image
//! states them somewhere a rebuild does not rewrite. A file's length is stated by
//! the tree of a supplementary descriptor, so an image holding one refuses a
//! length change; and the block a file sits at is stated by the record's own
//! system use area — which ISO 9660 leaves to a writer, and which this codec
//! keeps as it stands — so a record carrying one refuses a move. Replacing
//! contents, or growing within the room that follows a file, keeps both where
//! they were.

use super::Placement;
use super::metadata::{BLOCK_SIZE, Directory, Hex, IsoMetadata, Layout, Record};
use super::volume::{self, DESCRIPTOR_SECTOR, PRIMARY_FIELDS, SYSTEM_AREA_SIZE};
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use log::*;
use std::collections::{BTreeSet, HashMap};

/// The width of a Level 1 file name, and of a directory name.
const LEVEL1_NAME: usize = 8;

/// The width of a Level 1 file extension.
const LEVEL1_EXTENSION: usize = 3;

/// Rebuilds an image from the metadata and the files a decode produced.
///
/// `placement` decides how a file that no longer fits the blocks it had is given
/// room; see [`Placement`].
pub(crate) fn build(
  metadata: &IsoMetadata,
  files: &[&DecodedArtifact],
  placement: Placement,
) -> Result<Vec<u8>, CodecError> {
  let mut building = Building::new(&metadata.layout, placement)?;

  building.check_tree()?;

  for artifact in files {
    building.place(artifact)?;
  }

  building.check_carried(files)?;
  building.shift()?;
  building.translate()?;

  building.render(metadata, files)
}

/// One run of bytes a layout allocates, as [`Building::shift`] sees it.
///
/// The kind carries what re-laying out the sequence needs: which record states a
/// file's extent, which directory a directory is, and where the image states a
/// path table's block or records a run of uninterpreted bytes, so that a region
/// that moves takes every reference to it along.
///
/// The length is the region's own, in bytes: the runs a layout allocates do not
/// all fill their last block, and two of them can share one without sharing a
/// byte of it, which is what decides whether one has to give way to another.
#[derive(Debug, Clone, Copy)]
enum Region {
  File {
    directory: usize,
    record: usize,
    bytes: usize,
  },
  Directory {
    directory: usize,
    bytes: usize,
  },
  PathTable {
    which: Table,
    bytes: usize,
  },
  Opaque {
    run: usize,
    bytes: usize,
  },
}

impl Region {
  fn bytes(&self) -> usize {
    match self {
      Region::File { bytes, .. }
      | Region::Directory { bytes, .. }
      | Region::PathTable { bytes, .. }
      | Region::Opaque { bytes, .. } => *bytes,
    }
  }
}

/// Which of an image's path tables a region is: the two a rebuild writes. The
/// optional copies are opaque to a rebuild and are not part of the sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Table {
  Little,
  Big,
}

/// An image being rebuilt, and the room its structures leave free.
struct Building {
  layout: Layout,

  /// What the rebuild does with a file that outgrows the blocks it held.
  placement: Placement,

  /// Which blocks of the image are spoken for.
  space: Space,

  /// How many bytes the image held before the rebuild, which is what decides
  /// whether a file's contents lie outside it.
  original_len: usize,

  /// How many blocks the image held before the rebuild.
  original_sectors: usize,

  /// How many blocks the rebuilt image spans.
  sectors: usize,

  /// The files whose data lies beyond the image, by artifact path, as the image
  /// was read.
  ///
  /// Their records reach the blocks that follow the image, which a longer image
  /// pushes along, so [`Building::translate`] moves them with them. A file the
  /// rebuild itself places past the image's end is not one of them: it belongs
  /// to the longer image and stays where it was put.
  beyond: Vec<String>,

  /// The files whose extents straddle the image's end, by artifact path.
  ///
  /// One run of blocks across the end of the image cannot be kept whole by a
  /// longer image, so any of them is what an image that may not grow holds. Each
  /// counts as far as the image holds it and no further, since the blocks past
  /// the image's end belong to what follows it.
  straddling: BTreeSet<String>,

  /// Where each directory's entry sits in the path table.
  ///
  /// Found while the layout is still the image's own: a table lists identifiers
  /// rather than paths, so two directories in different places can carry the same
  /// name, and a rebuild that moves one would otherwise match the entry a move
  /// has just written.
  table_entries: Vec<Option<usize>>,
}

impl Building {
  fn new(layout: &Layout, placement: Placement) -> Result<Self, CodecError> {
    let content = layout.clone();
    let sectors = content.image_sectors as usize;

    // Both the length and the block map of the image being rebuilt are sized
    // from the metadata alone, so both are computed and reserved fallibly: a
    // metadata declaring an image this machine cannot hold is refused with an
    // error rather than aborting the process on an allocation that failed.
    let original_len = sectors.checked_mul(BLOCK_SIZE).ok_or_else(|| {
      CodecError::Message(format!(
        "The metadata declares an image of {} blocks, more than this machine can address",
        content.image_sectors
      ))
    })?;

    let table_entries = match_entries(&content);
    let mut building = Self {
      layout: content,
      placement,
      space: Space::new(sectors)?,
      original_len,
      original_sectors: sectors,
      sectors,
      beyond: Vec::new(),
      straddling: BTreeSet::new(),
      table_entries,
    };

    building.reserve()?;
    building.check_beyond()?;

    Ok(building)
  }

  /// Finds the files whose data lies beyond the image, and which of them a
  /// longer image would break.
  ///
  /// A file that starts at or past the image's end sits on the blocks that
  /// follow the track, which a longer image pushes along with everything else
  /// there: its record keeps its distance from the image's end, which
  /// [`Building::translate`] applies. One that starts inside the image and runs
  /// past its end is a single run of blocks across the boundary, and growing the
  /// image would insert new blocks into the middle of it, so it is recorded here
  /// and refuses growth.
  fn check_beyond(&mut self) -> Result<(), CodecError> {
    for directory in &self.layout.directories {
      for record in directory.records.iter().filter(|record| record.is_file()) {
        let extent = record.extent as usize;
        let blocks = blocks_for(record.length as usize);

        if extent >= self.original_sectors {
          if record.length > 0 {
            self
              .beyond
              .push(join(&directory.path, record.artifact_name()));
          }
        } else if extent + blocks > self.original_sectors {
          self
            .straddling
            .insert(join(&directory.path, record.artifact_name()));
        }
      }
    }

    if !self.beyond.is_empty() {
      debug!(
        "The image holds {} file(s) whose data lies beyond it, which a longer image moves with the \
         blocks that follow it: {}",
        self.beyond.len(),
        self.beyond.join(", ")
      );
    }

    Ok(())
  }

  /// Whether the image may grow past the blocks it held.
  ///
  /// A file whose extent straddles the image's end is one run of blocks across
  /// it, which a longer image would break in the middle of, so no policy grows
  /// for it. `preserve` goes further and will not lengthen an image that
  /// describes anything beyond its own end — a record that points past it, or a
  /// volume that claims blocks past it — because a longer image moves whatever is
  /// there.
  fn may_grow(&self) -> bool {
    if !self.straddling.is_empty() {
      return false;
    }

    match self.placement {
      Placement::Preserve => !self.holds_beyond(),
      Placement::Shift | Placement::Relocate => true,
    }
  }

  /// Whether the image describes anything beyond the blocks it holds.
  fn holds_beyond(&self) -> bool {
    !self.beyond.is_empty() || self.layout.volume_space_size as usize > self.original_sectors
  }

  /// Why the image may not grow, for an error that has to say.
  fn cannot_grow(&self) -> String {
    match self.straddling.iter().next() {
      Some(file) => format!(
        "the file '{file}' declares blocks on both sides of the image's end, so a longer image \
         would insert new blocks into the middle of it"
      ),
      None => {
        "the image describes data beyond its own end, which a longer image would move".to_owned()
      }
    }
  }

  /// Moves the records that describe data beyond the image with the blocks the
  /// image grew by.
  ///
  /// A block number in a record addresses the same space as the image's own
  /// length: the logical blocks of the track, in order. Blocks the image does not
  /// hold therefore begin at its end, and they are pushed along when the image
  /// grows, so a record that reaches them moves by the same number of blocks.
  /// The file it describes stays on the data it described, at the same distance
  /// from the image's end as before.
  ///
  /// Nothing else in the image states where those blocks are: a directory lies
  /// within the image, and the path tables and the descriptor state the blocks
  /// their own structures start at, which do not move.
  fn translate(&mut self) -> Result<(), CodecError> {
    let delta = self.sectors.saturating_sub(self.original_sectors);

    if delta == 0 || self.beyond.is_empty() {
      return Ok(());
    }

    let beyond: BTreeSet<String> = self.beyond.iter().cloned().collect();

    for directory in &mut self.layout.directories {
      for record in directory.records.iter_mut() {
        if !record.is_file() {
          continue;
        }

        let name = join(&directory.path, record.artifact_name());

        if !beyond.contains(&name) {
          continue;
        }

        record.extent = u32::try_from(u64::from(record.extent) + delta as u64).map_err(|_| {
          CodecError::Message(format!(
            "The image grew by {delta} blocks, which would carry the file '{name}', whose data lies \
             beyond it, past the last block ISO 9660 can address"
          ))
        })?;

        debug!(
          "The file '{name}', whose data lies beyond the image, moves {delta} blocks later with the \
           blocks the image grew by: it sits at block {}",
          record.extent
        );
      }
    }

    Ok(())
  }

  /// Re-lays out the image's regions in their own order, which is what a `shift`
  /// rebuild asks for.
  ///
  /// The regions are one sequence of blocks, so a region that outgrew the room
  /// before the next one pushes the whole tail along by exactly what it needs and
  /// by nothing more: every region keeps its place in the sequence, no region is
  /// relocated, and an image nothing grew in is written back byte for byte. The
  /// image grows only when the last region runs past its end, which
  /// [`Building::translate`] then answers for.
  ///
  /// The regions the pass cannot push are the System Area and the descriptor set
  /// at the very front: nothing the rebuild can grow precedes them.
  ///
  /// The room [`Building::space`] records is not consulted here: this is the last
  /// pass that moves anything, and [`Building::place`] has already claimed the
  /// blocks each file's own extent grew into.
  fn shift(&mut self) -> Result<(), CodecError> {
    if self.placement != Placement::Shift {
      return Ok(());
    }

    let mut regions = self.regions();

    regions.sort_by_key(|(start, _)| *start);

    // How far the tail of the sequence has been pushed, in whole blocks: the
    // regions move in blocks, since that is the unit a record states.
    let mut pushed = 0usize;
    let mut moved: Vec<(usize, Region)> = Vec::new();
    let mut last = 0usize;

    for (index, (start, region)) in regions.iter().enumerate() {
      let end = start + pushed * BLOCK_SIZE + region.bytes();

      if pushed > 0 {
        moved.push((pushed, *region));
      }

      last = end;

      // Only as far as the next region's own bytes need: the blocks between the
      // two are left as they are, and two regions that share a block without
      // sharing a byte of it — a run the codec preserves ending where the next
      // one begins — push nothing at all.
      if let Some((next, _)) = regions.get(index + 1) {
        let over = end.saturating_sub(*next);

        pushed = pushed.max(over.div_ceil(BLOCK_SIZE));
      }
    }

    let sectors = self.sectors.max(last.div_ceil(BLOCK_SIZE));

    if moved.is_empty() && sectors == self.sectors {
      return Ok(());
    }

    if sectors > self.original_sectors && !self.may_grow() {
      return Err(CodecError::Message(format!(
        "Re-laying the image out in the order it was built would lengthen it, and it may not grow: {}",
        self.cannot_grow()
      )));
    }

    // A directory or a path table that moves leaves the second copy of the path
    // tables describing where it was, and a rebuild does not rewrite that copy.
    if self.layout.path_table.has_copy()
      && moved
        .iter()
        .any(|(_, region)| matches!(region, Region::Directory { .. } | Region::PathTable { .. }))
    {
      return Err(CodecError::Message(
        "Re-laying the image out in the order it was built would move a directory or a path table, \
         and the image holds a second copy of its path tables that a rebuild does not rewrite"
          .to_owned(),
      ));
    }

    for (delta, region) in moved {
      self.move_region(region, delta)?;
    }

    if sectors != self.sectors {
      debug!("The image grows to {sectors} blocks to hold the regions a grown file pushed along");

      self.sectors = sectors;
    }

    Ok(())
  }

  /// The regions a layout allocates, with the byte each one starts at.
  ///
  /// Every file, directory, path table and run of uninterpreted bytes, which is
  /// everything the image is made of whose bytes a rebuild knows the meaning of.
  /// The System Area and the descriptor set are left out: they hold no extent of
  /// their own, and nothing the rebuild can grow precedes them.
  ///
  /// A record whose data lies beyond the image is left out — it belongs to what
  /// follows the image, which [`Building::translate`] carries rather than pushes —
  /// and one that runs across the image's end counts only as far as the image
  /// holds it, since the rest belongs to what follows the image too.
  fn regions(&self) -> Vec<(usize, Region)> {
    let mut regions = Vec::new();

    for (index, directory) in self.layout.directories.iter().enumerate() {
      let start = directory.extent as usize * BLOCK_SIZE;

      if directory.length > 0 && start < self.original_len {
        regions.push((
          start,
          Region::Directory {
            directory: index,
            bytes: directory.length as usize,
          },
        ));
      }

      for (record, held) in directory.records.iter().enumerate() {
        if !held.is_file() || held.length == 0 {
          continue;
        }

        let start = held.extent as usize * BLOCK_SIZE;
        let name = join(&directory.path, held.artifact_name());

        // A file whose data lies beyond the image belongs to what follows the
        // image, which [`Building::translate`] carries rather than this pass.
        if self.beyond.contains(&name) {
          continue;
        }

        // One that runs across the image's end counts only as far as the image
        // holds it: the rest belongs to what follows the image, and no policy
        // grows this one for it.
        let bytes = if self.straddling.contains(&name) {
          self.original_len.saturating_sub(start)
        } else {
          held.length as usize
        };

        if bytes == 0 {
          continue;
        }

        regions.push((
          start,
          Region::File {
            directory: index,
            record,
            bytes,
          },
        ));
      }
    }

    let tables = self.layout.path_table.size();

    // Only the two tables a rebuild writes. The optional copies are not listed:
    // they are opaque to a rebuild — preserved as the runs of bytes no
    // structure accounts for — and listing them here as well would give those
    // same bytes two regions, whose meeting reads as a collision and pushes the
    // sequence along for an image nothing grew in. A rebuild never moves a
    // directory or a table while those copies exist (see [`Building::shift`]),
    // so they have no place in the sequence either way.
    for which in [
      Table::Little,
      Table::Big,
    ] {
      let start = *self.path_table_sector_for(which) as usize * BLOCK_SIZE;

      if start == 0 || tables == 0 || start >= self.original_len {
        continue;
      }

      regions.push((
        start,
        Region::PathTable {
          which,
          bytes: tables,
        },
      ));
    }

    for (run, held) in self.layout.opaque.iter().enumerate() {
      let Some(start) = usize::try_from(held.offset).ok() else {
        continue;
      };

      let bytes = held.data.len();

      if bytes == 0 || start >= self.original_len {
        continue;
      }

      regions.push((
        start,
        Region::Opaque {
          run,
          bytes: bytes.min(self.original_len - start),
        },
      ));
    }

    regions
  }

  /// Moves one region to where the sequence puts it, and every reference to it
  /// the codec knows how to rewrite.
  fn move_region(&mut self, region: Region, delta: usize) -> Result<(), CodecError> {
    match region {
      Region::File {
        directory, record, ..
      } => {
        let path = self.layout.directories[directory].path.clone();
        let held = &mut self.layout.directories[directory].records[record];
        let name = join(&path, held.artifact_name());

        held.extent = moved_block(held.extent, delta, &format!("The file '{name}'"))?;
      }
      Region::Directory { directory, .. } => {
        let name = display_path(&self.layout.directories[directory].path);
        let extent = moved_block(
          self.layout.directories[directory].extent,
          delta,
          &format!("The directory '{name}'"),
        )?;

        self.set_extent(directory, extent);
      }
      Region::PathTable { which, .. } => {
        let sector = self.path_table_sector(which);

        *sector = moved_block(*sector, delta, "The image's path tables")?;
      }
      Region::Opaque { run, .. } => {
        let held = &mut self.layout.opaque[run];
        let moved = u64::try_from(delta)
          .ok()
          .and_then(|delta| delta.checked_mul(BLOCK_SIZE as u64))
          .ok_or_else(|| {
            CodecError::Message(
              "Re-laying the image out would move the bytes it holds without interpreting them \
               further than an image can state"
                .to_owned(),
            )
          })?;

        held.offset = held.offset.checked_add(moved).ok_or_else(|| {
          CodecError::Message(format!(
            "The {} bytes the image holds at offset {} without interpreting them would move past \
             the end of what an image can state",
            held.data.len(),
            held.offset
          ))
        })?;
      }
    }

    Ok(())
  }

  /// The descriptor field stating where one of the path tables starts.
  fn path_table_sector(&mut self, which: Table) -> &mut u32 {
    match which {
      Table::Little => &mut self.layout.path_table.l_sector,
      Table::Big => &mut self.layout.path_table.m_sector,
    }
  }

  /// The same, for a layout the pass only reads.
  fn path_table_sector_for(&self, which: Table) -> &u32 {
    match which {
      Table::Little => &self.layout.path_table.l_sector,
      Table::Big => &self.layout.path_table.m_sector,
    }
  }

  /// States a directory's new extent everywhere the image states it: in the
  /// directory's own record, in the record its parent holds for it, and in the
  /// path table entry that names it.
  ///
  /// The three have to agree: a reader that follows a record and one that walks a
  /// path table have to arrive at the same blocks, and the descriptor's record for
  /// the root takes its extent from the root's own record.
  ///
  /// Nothing here is found by the extent it holds: another directory may have been
  /// moved to the block this one is leaving, and a rebuild that has to move
  /// several would then state one of them twice.
  fn set_extent(&mut self, directory: usize, extent: u32) {
    let path = self.layout.directories[directory].path.clone();
    let leaf = path.rsplit('/').next().unwrap_or("").to_owned();
    let parent = parent_of(&path).to_owned();

    self.layout.directories[directory].extent = extent;

    // The entry the directory holds for itself.
    if let Some(own) = self.layout.directories[directory]
      .records
      .first_mut()
      .filter(|record| record.name == "\u{0}")
    {
      own.extent = extent;
    }

    // The entry its parent holds for it, which carries the name it has there: the
    // root's is the record it holds for its own parent, which is itself.
    let held_by_parent = if path.is_empty() {
      "\u{1}"
    } else {
      leaf.as_str()
    };

    for held in &mut self.layout.directories {
      if held.path != parent {
        continue;
      }

      for record in held
        .records
        .iter_mut()
        .filter(|record| record.is_directory() && record.name == held_by_parent)
      {
        record.extent = extent;
      }
    }

    let entry = self.table_entries.get(directory).copied().flatten();

    if let Some(entry) = entry.and_then(|entry| self.layout.path_table.entries.get_mut(entry)) {
      entry.extent = extent;
    }
  }

  /// Marks the blocks the layout's own structures occupy.
  fn reserve(&mut self) -> Result<(), CodecError> {
    self.space.reserve_range(0, SYSTEM_AREA_SIZE);
    self.space.reserve_range(
      DESCRIPTOR_SECTOR as usize * BLOCK_SIZE,
      (DESCRIPTOR_SECTOR as usize + 1) * BLOCK_SIZE,
    );

    let tables = self.layout.path_table.size();

    self.space.reserve_range(
      self.layout.path_table.l_sector as usize * BLOCK_SIZE,
      self.layout.path_table.l_sector as usize * BLOCK_SIZE + tables,
    );
    self.space.reserve_range(
      self.layout.path_table.m_sector as usize * BLOCK_SIZE,
      self.layout.path_table.m_sector as usize * BLOCK_SIZE + tables,
    );

    for directory in &self.layout.directories {
      let start = directory.extent as usize * BLOCK_SIZE;

      self
        .space
        .reserve_range(start, start + directory.length as usize);
    }

    for (offset, bytes) in self.layout.opaque_runs() {
      let start = usize::try_from(offset).unwrap_or(usize::MAX);

      self.space.reserve_range(start, start + bytes.len());
    }

    let files: Vec<(usize, usize)> = self
      .layout
      .directories
      .iter()
      .flat_map(|directory| &directory.records)
      .filter(|record| record.is_file())
      .map(|record| (record.extent as usize, record.length as usize))
      .collect();

    for (extent, length) in files {
      let start = extent * BLOCK_SIZE;

      self.space.reserve_range(start, start + length);
    }

    Ok(())
  }

  /// Checks that the layout describes an image this codec can write.
  fn check_tree(&self) -> Result<(), CodecError> {
    if self.layout.root().is_none() {
      return Err(CodecError::Message(
        "The ISO metadata records no root directory, so there is no filesystem to rebuild"
          .to_owned(),
      ));
    }

    for directory in &self.layout.directories {
      let Some(own) = directory.own_record() else {
        return Err(CodecError::Message(format!(
          "The layout's directory '{}' holds no record for itself",
          display_path(&directory.path)
        )));
      };

      if own.extent != directory.extent || own.length != directory.length {
        return Err(CodecError::Message(format!(
          "The layout's directory '{}' declares an extent of {} blocks for {} bytes, and its own \
           record says {} for {}",
          display_path(&directory.path),
          directory.extent,
          directory.length,
          own.extent,
          own.length
        )));
      }

      for record in directory
        .records
        .iter()
        .filter(|record| record.is_directory())
      {
        if record.name == "\u{0}" || record.name == "\u{1}" {
          continue;
        }

        if !self
          .layout
          .directories
          .iter()
          .any(|held| held.extent == record.extent)
        {
          return Err(CodecError::Message(format!(
            "The layout records the directory '{}' at block {}, which it does not hold",
            join(&directory.path, &record.name),
            record.extent
          )));
        }
      }
    }

    Ok(())
  }

  /// Puts one artifact's contents into the image.
  fn place(&mut self, artifact: &DecodedArtifact) -> Result<(), CodecError> {
    let length = artifact.data.len();

    if length > u32::MAX as usize {
      return Err(CodecError::Message(format!(
        "'{}' holds {length} bytes, more than an ISO 9660 record can describe",
        artifact.name
      )));
    }

    let Some((directory, index)) = self.find(&artifact.name) else {
      return self.add(artifact);
    };

    let (extent, recorded, carried, has_system_use) = {
      let record = &self.layout.directories[directory].records[index];

      (
        record.extent as usize,
        record.length as usize,
        record.in_image(self.original_len),
        !record.system_use.is_empty(),
      )
    };

    if carried < recorded {
      // The record points at blocks the image does not hold: the file lives on
      // the track that follows, and only its record is the codec's to write.
      if length != carried {
        return Err(CodecError::Message(format!(
          "The contents of '{}' lie outside the image, in the {} bytes of the {recorded} its \
           record declares, so they cannot be replaced by {length} others",
          artifact.name, carried
        )));
      }

      return Ok(());
    }

    if length == recorded {
      return Ok(());
    }

    let blocks = blocks_for(length);
    let was = blocks_for(recorded);

    self.check_resized(&artifact.name)?;

    let placed = if blocks <= was {
      // It still fits the blocks it had, so nothing else has to move. A file that
      // shrank keeps the room it no longer fills: `shift` re-lays out what a file
      // that grew pushed along, not what one gave back.
      extent
    } else if self.placement == Placement::Shift {
      // It grows where it was, and `shift` pushes along whatever follows it:
      // nothing is relocated, and the image keeps its order. Whether the room is
      // there is the shift pass's business, not this one's.
      self.space.mark_range(extent + was, extent + blocks)?;

      extent
    } else if self.space.free_run(extent + was, blocks - was) {
      // The blocks after its own are free, so it grows where it was: nothing
      // that seeks to its block notices.
      self.space.mark_range(extent + was, extent + blocks)?;

      extent
    } else if self.may_grow() && self.space.free_to_the_end(extent + was) {
      // Nothing sits after it, so it grows where it was into the room past the
      // image's end: the image grows with it, and the file stays where it was.
      self.grow(extent + blocks)?;
      self.space.mark_range(extent + was, extent + blocks)?;

      debug!(
        "The image grows to {} blocks to hold '{}', which stays where it was",
        self.sectors, artifact.name
      );

      extent
    } else if self.placement == Placement::Preserve {
      return Err(CodecError::Message(format!(
        "'{}' no longer fits the {was} blocks it held at block {extent}, and the room that follows it \
         is taken: 'preserve' leaves every extent where it is",
        artifact.name
      )));
    } else {
      let placed = self.allocate(blocks, &artifact.name)?;

      self.check_moved(&artifact.name, has_system_use)?;
      self.space.free_range(extent, extent + was);

      warn!(
        "The file '{}' no longer fits the {was} blocks it held at block {extent}: it moves to \
         block {placed}, which a reference to its block would notice",
        artifact.name
      );

      placed
    };

    let record = &mut self.layout.directories[directory].records[index];

    record.extent = u32::try_from(placed).map_err(|_| {
      CodecError::Message(format!(
        "'{}' would sit past the last block ISO 9660 can address",
        artifact.name
      ))
    })?;
    record.length = length as u32;

    Ok(())
  }

  /// Adds a file the image does not hold yet.
  fn add(&mut self, artifact: &DecodedArtifact) -> Result<(), CodecError> {
    let (parent, leaf) = split_path(&artifact.name);

    let Some(directory) = self
      .layout
      .directories
      .iter()
      .position(|directory| directory.path == parent)
    else {
      return Err(CodecError::Message(format!(
        "The image holds no directory '{parent}' to add '{}' to",
        artifact.name
      )));
    };

    validate_file_name(leaf).map_err(|reason| {
      CodecError::Message(format!(
        "'{}' cannot be written as plain ISO 9660 Level 1: {reason}",
        artifact.name
      ))
    })?;

    let taken = self.layout.directories[directory]
      .records
      .iter()
      .any(|record| record.is_directory() && record.name == leaf);

    if taken {
      return Err(CodecError::Message(format!(
        "The image holds a directory '{}', so '{}' cannot be added beside it",
        artifact.name, artifact.name
      )));
    }

    self.refuse_tree_change(&artifact.name)?;

    let extent = self.allocate(blocks_for(artifact.data.len()), &artifact.name)?;

    let (date, sequence) = {
      let own = self.layout.directories[directory].own_record();

      (
        own.map(|record| record.date.clone()).unwrap_or_default(),
        own.map(|record| record.volume_sequence_number).unwrap_or(1),
      )
    };

    let record = Record {
      name: format!("{leaf};1"),
      extent: u32::try_from(extent).map_err(|_| {
        CodecError::Message(format!(
          "'{}' would sit past the last block ISO 9660 can address",
          artifact.name
        ))
      })?,
      length: artifact.data.len() as u32,
      flags: 0,
      date,
      volume_sequence_number: sequence,
      system_use: Hex::default(),
      pad: Hex::default(),
    };

    self.append(directory, record, &artifact.name)
  }

  /// Appends a record to a directory, growing the directory if it has to.
  fn append(&mut self, directory: usize, record: Record, name: &str) -> Result<(), CodecError> {
    self.layout.directories[directory].records.push(record);

    let used = records_len(&self.layout.directories[directory].records)?;
    let length = self.layout.directories[directory].length as usize;

    if used <= length {
      return Ok(());
    }

    // A directory's extent can only take the blocks that follow it: nothing
    // points at it by name, so moving it would leave every record that does
    // describe where its files used to be.
    let blocks = blocks_for(used);
    let extent = self.layout.directories[directory].extent as usize;
    let from = extent + blocks_for(length);

    // Under `shift` the regions that follow are pushed along, so the directory
    // takes them whether or not something holds them.
    if self.placement != Placement::Shift && !self.space.free_run(from, blocks - blocks_for(length))
    {
      return Err(CodecError::Message(format!(
        "The directory '{}' has no room for '{name}': its records take {used} bytes and the blocks \
         that follow its {} are taken",
        display_path(&self.layout.directories[directory].path),
        self.layout.directories[directory].length
      )));
    }

    self.space.mark_range(from, extent + blocks)?;

    let length = (blocks * BLOCK_SIZE) as u32;

    self.layout.directories[directory].length = length;

    self.set_length(directory, length);

    Ok(())
  }

  /// Records a directory's new length where the image states it: in the
  /// directory's own record and in the record its parent holds for it.
  ///
  /// The parent's record is found by the extent it declares, which names this
  /// directory and no other while every extent is still the one the image
  /// held: this runs under [`Building::place`], before [`Building::shift`]
  /// re-lays anything out, and a decode refuses an image that gives two
  /// directories one extent. Once directories can move, an extent no longer
  /// names the directory it came from — which is why [`Building::set_extent`],
  /// the pass that moves directories, finds the parent's record by identifier
  /// instead.
  fn set_length(&mut self, directory: usize, length: u32) {
    if let Some(own) = self.layout.directories[directory]
      .records
      .first_mut()
      .filter(|record| record.name == "\u{0}")
    {
      own.length = length;
    }

    let path = self.layout.directories[directory].path.clone();

    if path.is_empty() {
      return;
    }

    let extent = self.layout.directories[directory].extent;

    if let Some(parent) = self
      .layout
      .directories
      .iter_mut()
      .find(|held| held.path == parent_of(&path))
    {
      for record in parent
        .records
        .iter_mut()
        .filter(|record| record.is_directory() && record.extent == extent)
      {
        record.length = length;
      }
    }
  }

  /// The record carrying the artifact of a given name, if the layout has one.
  fn find(&self, name: &str) -> Option<(usize, usize)> {
    self
      .layout
      .directories
      .iter()
      .enumerate()
      .find_map(|(directory, held)| {
        held
          .records
          .iter()
          .position(|record| record.is_file() && join(&held.path, record.artifact_name()) == name)
          .map(|index| (directory, index))
      })
  }

  /// Finds room for a file: the first free room in the image, and room after the
  /// image's end when it has none and may grow.
  fn allocate(&mut self, blocks: usize, name: &str) -> Result<usize, CodecError> {
    if blocks == 0 {
      return Ok(0);
    }

    if let Some(from) = self.space.first_run(blocks) {
      self.space.mark_range(from, from + blocks)?;

      return Ok(from);
    }

    if self.may_grow() {
      let tail = self.space.tail_run();
      let from = self.space.blocks() - tail;

      self.grow(from + blocks)?;
      self.space.mark_range(from, from + blocks)?;

      debug!("The image grows to {} blocks to hold a file", self.sectors);

      return Ok(from);
    }

    Err(CodecError::Message(format!(
      "The image leaves no run of {blocks} free blocks for '{name}', and it may not grow: {}",
      self.cannot_grow()
    )))
  }

  fn grow(&mut self, blocks: usize) -> Result<(), CodecError> {
    if blocks > self.sectors {
      self.sectors = blocks;
      self.space.resize(blocks)?;
    }

    Ok(())
  }

  /// The Volume Space Size to write for the rebuilt image, which is how many
  /// logical blocks the volume it belongs to spans.
  ///
  /// The image's own length and that size are numbers in one space, so a volume
  /// that declared blocks past the image's end declared blocks on what follows
  /// the track: they move along with the image, and so does the size. An image
  /// that did not grow moved nothing, so the size stays as the image declared it
  /// — it may cover more than the image holds, and read back as it stands. A
  /// volume that ended within its image had nothing past its end to keep, and the
  /// blocks the image gained are its own, so it grows to the image's new end.
  fn volume_space_size(&self, sectors: u32) -> Result<u32, CodecError> {
    let declared = self.layout.volume_space_size;

    if self.sectors == self.original_sectors {
      // Nothing grew, so nothing moved: the volume keeps the size it declared,
      // which may cover more than the image holds.
      return Ok(declared);
    }

    if declared <= self.layout.image_sectors {
      return Ok(declared.max(sectors));
    }

    let grew = u32::try_from(self.sectors.saturating_sub(self.original_sectors)).map_err(|_| {
      CodecError::Message("The image grew by more blocks than ISO 9660 can describe".to_owned())
    })?;

    declared.checked_add(grew).ok_or_else(|| {
      CodecError::Message(format!(
        "The volume declares {declared} blocks, and the image grew by {grew}, more than ISO 9660 can \
         describe"
      ))
    })
  }

  /// Refuses a change to a file's length where the image states that length
  /// somewhere a rebuild does not rewrite.
  ///
  /// A supplementary descriptor describes a second tree over the same files,
  /// whose records state each file's extent and length, and this codec keeps
  /// those records as they stand.
  fn check_resized(&self, name: &str) -> Result<(), CodecError> {
    if self.layout.secondary_descriptors {
      return Err(CodecError::Message(format!(
        "The image holds a volume descriptor besides its primary one, whose tree states where each \
         of its files sits and how long it is, and a rebuild does not rewrite it, so the length of \
         '{name}' cannot change"
      )));
    }

    Ok(())
  }

  /// Refuses a move that would leave a structure the codec does not rewrite
  /// describing where a file used to be.
  fn check_moved(&self, name: &str, has_system_use: bool) -> Result<(), CodecError> {
    if self.layout.secondary_descriptors {
      return Err(CodecError::Message(format!(
        "'{name}' no longer fits where it was, and the image holds a volume descriptor besides its \
         primary one whose tree a rebuild would not rewrite"
      )));
    }

    if self.layout.path_table.has_copy() {
      return Err(CodecError::Message(format!(
        "'{name}' no longer fits where it was, and the image holds a second copy of its path \
         tables that a rebuild would not rewrite"
      )));
    }

    if has_system_use {
      return Err(CodecError::Message(format!(
        "'{name}' no longer fits where it was, and the record for it carries a system use area — an \
         extension this codec preserves without interpreting — that may describe where it sits"
      )));
    }

    Ok(())
  }

  /// Refuses a change to the tree of an image whose other structures a rebuild
  /// does not rewrite.
  fn refuse_tree_change(&self, name: &str) -> Result<(), CodecError> {
    if self.layout.secondary_descriptors {
      return Err(CodecError::Message(format!(
        "The image holds a volume descriptor besides its primary one, whose tree a rebuild does not \
         rewrite, so '{name}' cannot be added to it"
      )));
    }

    if self.layout.path_table.has_copy() {
      return Err(CodecError::Message(format!(
        "The image holds a second copy of its path tables, which a rebuild does not rewrite, so \
         '{name}' cannot be added to it"
      )));
    }

    Ok(())
  }

  /// Checks that every file the image holds is carried by an artifact.
  fn check_carried(&self, files: &[&DecodedArtifact]) -> Result<(), CodecError> {
    let names: BTreeSet<&str> = files.iter().map(|file| file.name.as_str()).collect();

    for directory in &self.layout.directories {
      for record in directory.records.iter().filter(|record| record.is_file()) {
        if record.in_image(self.original_len) < record.length as usize {
          continue;
        }

        let name = join(&directory.path, record.artifact_name());

        if !names.contains(name.as_str()) {
          return Err(CodecError::Message(format!(
            "The layout holds the file '{name}', and no artifact carries its contents"
          )));
        }
      }
    }

    Ok(())
  }

  /// Writes the image.
  fn render(
    &self,
    metadata: &IsoMetadata,
    files: &[&DecodedArtifact],
  ) -> Result<Vec<u8>, CodecError> {
    // The rebuilt image's length comes from the metadata the rebuild starts
    // from and the files it is given, so it is computed and reserved the same
    // way as the rest of the image's dimensions: checked, fallible, and never
    // an allocation this process dies in.
    let length = self.sectors.checked_mul(BLOCK_SIZE).ok_or_else(|| {
      CodecError::Message(
        "The rebuilt image spans more blocks than this machine can address".to_owned(),
      )
    })?;

    let mut image = Vec::new();

    image.try_reserve_exact(length).map_err(|_| {
      CodecError::Message(format!(
        "A rebuilt image of {length} bytes is more than this machine can hold"
      ))
    })?;

    image.resize(length, 0);

    let mut written: Vec<(usize, usize, String)> = Vec::new();

    let system_area = self.layout.system_area.as_bytes();

    if system_area.len() > SYSTEM_AREA_SIZE {
      return Err(CodecError::Message(format!(
        "The ISO metadata records a System Area of {} bytes, and ISO 9660 reserves {SYSTEM_AREA_SIZE}",
        system_area.len()
      )));
    }

    image[..system_area.len()].copy_from_slice(system_area);
    written.push((0, SYSTEM_AREA_SIZE, "the image's System Area".to_owned()));

    let root = self
      .layout
      .root()
      .and_then(Directory::own_record)
      .ok_or_else(|| {
        CodecError::Message("The ISO metadata records no root directory".to_owned())
      })?;

    let mut root = root.clone();
    root.system_use = Hex::default();

    let sectors = u32::try_from(self.sectors).map_err(|_| {
      CodecError::Message(
        "The rebuilt image spans more blocks than ISO 9660 can address".to_owned(),
      )
    })?;

    let descriptor = volume::serialize_primary(
      &self.layout,
      &metadata.volume_identifier,
      &root,
      self.volume_space_size(sectors)?,
    )?;
    let start = DESCRIPTOR_SECTOR as usize * BLOCK_SIZE;

    image[start..start + BLOCK_SIZE].copy_from_slice(&descriptor);

    for (what, offset, width) in PRIMARY_FIELDS {
      written.push((
        start + offset,
        start + offset + width,
        format!("the descriptor's {what}"),
      ));
    }

    for (sector, big_endian) in [
      (self.layout.path_table.l_sector, false),
      (self.layout.path_table.m_sector, true),
    ] {
      let table = volume::serialize_path_table(&self.layout.path_table, big_endian);
      let start = sector as usize * BLOCK_SIZE;

      if start + table.len() > image.len() {
        return Err(CodecError::Message(format!(
          "The image's path tables at block {sector} run past the end of the image"
        )));
      }

      image[start..start + table.len()].copy_from_slice(&table);
      written.push((
        start,
        start + table.len(),
        "the image's path tables".to_owned(),
      ));
    }

    for directory in &self.layout.directories {
      let bytes = volume::serialize_directory(directory)?;
      let start = directory.extent as usize * BLOCK_SIZE;

      if start + bytes.len() > image.len() {
        return Err(CodecError::Message(format!(
          "The directory '{}' runs past the end of the image",
          display_path(&directory.path)
        )));
      }

      image[start..start + bytes.len()].copy_from_slice(&bytes);

      for (offset, size) in volume::record_offsets(&directory.records)? {
        written.push((
          start + offset,
          start + offset + size,
          format!("the directory '{}'", display_path(&directory.path)),
        ));
      }
    }

    let held: HashMap<&str, &[u8]> = files
      .iter()
      .map(|file| (file.name.as_str(), file.data.as_slice()))
      .collect();

    for directory in &self.layout.directories {
      for record in directory.records.iter().filter(|record| record.is_file()) {
        let name = join(&directory.path, record.artifact_name());

        let Some(data) = held.get(name.as_str()) else {
          continue;
        };

        // A file whose contents the image does not hold in full: the record is
        // written, and as much of the file as lies inside the image with it.
        let length = data.len().min(record.in_image(image.len()));
        let start = record.extent as usize * BLOCK_SIZE;

        if length == 0 {
          continue;
        }

        if start + length > image.len() {
          return Err(CodecError::Message(format!(
            "The file '{name}' runs past the end of the image"
          )));
        }

        // A hand-edited layout may point two records at the same blocks, or a
        // file at a structure the image holds elsewhere. Neither a decode nor
        // a placement policy ever produces one — the decode refuses two
        // structures in the same bytes and the policies keep them apart — so
        // an overlap here is a layout that disagrees with itself, and writing
        // over what stands there would lose it silently.
        if let Some((_, _, what)) = written
          .iter()
          .find(|(held, last, _)| start < *last && *held < start + length)
        {
          return Err(CodecError::Message(format!(
            "The layout places '{name}' at offset {start} over {what}, and a rebuild cannot \
             write both"
          )));
        }

        image[start..start + length].copy_from_slice(&data[..length]);
        written.push((start, start + length, format!("the file '{name}'")));
      }
    }

    for (offset, bytes) in self.layout.opaque_runs() {
      let start = usize::try_from(offset).map_err(|_| {
        CodecError::Message("The ISO metadata records bytes beyond the image".to_owned())
      })?;
      let end = start + bytes.len();

      if end > image.len() {
        return Err(CodecError::Message(format!(
          "The ISO metadata records {} bytes at offset {start}, which the image is too short to hold",
          bytes.len()
        )));
      }

      if let Some((_, _, what)) = written
        .iter()
        .find(|(held, last, _)| start < *last && *held < end)
      {
        return Err(CodecError::Message(format!(
          "The image holds {} bytes at offset {start} that the codec does not interpret, and the \
           rebuild would write over them with {what}",
          bytes.len()
        )));
      }

      image[start..end].copy_from_slice(&bytes);
    }

    Ok(image)
  }
}

/// Which blocks of an image are spoken for.
struct Space {
  used: Vec<bool>,
}

impl Space {
  fn new(blocks: usize) -> Result<Self, CodecError> {
    let mut space = Self { used: Vec::new() };

    space.reserve(blocks)?;
    space.used.resize(blocks, false);

    Ok(space)
  }

  fn blocks(&self) -> usize {
    self.used.len()
  }

  fn resize(&mut self, blocks: usize) -> Result<(), CodecError> {
    self.reserve(blocks)?;
    self.used.resize(blocks, false);

    Ok(())
  }

  /// Takes room for the image's whole current length, refused when this
  /// machine has none to give rather than aborting the process on it.
  fn reserve(&mut self, blocks: usize) -> Result<(), CodecError> {
    if blocks <= self.used.len() {
      return Ok(());
    }

    self
      .used
      .try_reserve_exact(blocks - self.used.len())
      .map_err(|_| {
        CodecError::Message(format!(
          "An image of {blocks} logical blocks is more than this machine can hold"
        ))
      })
  }

  fn reserve_range(&mut self, from: usize, to: usize) {
    let from = (from / BLOCK_SIZE).min(self.used.len());
    let to = to.div_ceil(BLOCK_SIZE).min(self.used.len());

    for block in &mut self.used[from..to] {
      *block = true;
    }
  }

  fn mark_range(&mut self, from: usize, to: usize) -> Result<(), CodecError> {
    if to > self.used.len() {
      self.resize(to)?;
    }

    for block in &mut self.used[from..to] {
      *block = true;
    }

    Ok(())
  }

  fn free_range(&mut self, from: usize, to: usize) {
    let from = (from / BLOCK_SIZE).min(self.used.len());
    let to = to.div_ceil(BLOCK_SIZE).min(self.used.len());

    for block in &mut self.used[from..to] {
      *block = false;
    }
  }

  /// Whether every block in a range is free.
  ///
  /// Blocks past the image's end are not free: a file judged to fit there would
  /// leave its record pointing at blocks the image does not hold, which a reader
  /// would take for a file of its declared length rather than notice.
  fn free_run(&self, from: usize, blocks: usize) -> bool {
    from + blocks <= self.used.len() && self.used[from..from + blocks].iter().all(|used| !*used)
  }

  /// Whether every block from one on is free, to the image's end.
  fn free_to_the_end(&self, from: usize) -> bool {
    self.used[from.min(self.used.len())..]
      .iter()
      .all(|used| !*used)
  }

  fn first_run(&self, blocks: usize) -> Option<usize> {
    let mut run = 0;

    for (block, used) in self.used.iter().enumerate() {
      if *used {
        run = 0;
        continue;
      }

      run += 1;

      if run >= blocks {
        return Some(block + 1 - blocks);
      }
    }

    None
  }

  /// How many blocks at the end of the image are free.
  fn tail_run(&self) -> usize {
    self.used.iter().rev().take_while(|block| !**block).count()
  }
}

/// How many blocks a file of a given length takes.
fn blocks_for(length: usize) -> usize {
  length.div_ceil(BLOCK_SIZE)
}

/// Where each directory's entry sits in a layout's path table, by the block the
/// directory is at and the name the entry carries there.
///
/// A directory a table does not name, or one whose name and block two entries
/// claim, is left out: the image's table and its tree already disagree, and a
/// rebuild leaves them both as they stand rather than guess which is which.
fn match_entries(layout: &Layout) -> Vec<Option<usize>> {
  layout
    .directories
    .iter()
    .map(|directory| {
      let leaf = directory.path.rsplit('/').next().unwrap_or("");
      let named = if leaf.is_empty() { "\u{0}" } else { leaf };

      let mut found = None;
      let mut count = 0;

      for (index, entry) in layout.path_table.entries.iter().enumerate() {
        if entry.extent == directory.extent && entry.name == named {
          found = Some(index);
          count += 1;
        }
      }

      if count == 1 { found } else { None }
    })
    .collect()
}

/// A block number the rebuild moved `delta` blocks later, or a refusal naming
/// what would have moved past the end of the address space.
fn moved_block(block: u32, delta: usize, what: &str) -> Result<u32, CodecError> {
  u32::try_from(u64::from(block) + delta as u64).map_err(|_| {
    CodecError::Message(format!(
      "{what} would move past the last block ISO 9660 can address"
    ))
  })
}

/// How many bytes a directory's records take, laid out as a rebuild writes them.
fn records_len(records: &[Record]) -> Result<usize, CodecError> {
  let offsets = volume::record_offsets(records)?;

  Ok(offsets.last().map_or(0, |(offset, size)| offset + size))
}

/// The directory an artifact path sits in, and the name it carries there.
fn split_path(path: &str) -> (&str, &str) {
  match path.rsplit_once('/') {
    Some((parent, leaf)) => (parent, leaf),
    None => ("", path),
  }
}

/// The directory a path sits in.
fn parent_of(path: &str) -> &str {
  path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// A path below a directory.
fn join(path: &str, name: &str) -> String {
  if path.is_empty() {
    name.to_owned()
  } else {
    format!("{path}/{name}")
  }
}

/// How a directory is named in a message, the root having no path of its own.
fn display_path(path: &str) -> String {
  if path.is_empty() {
    "/".to_owned()
  } else {
    format!("/{path}")
  }
}

/// Validates an ISO 9660 Level 1 file name.
///
/// Must have a non-empty base name of at most 8 characters and an extension
/// of at most 3 characters. Both may contain only A-Z, 0-9 and _.
pub(crate) fn validate_file_name(name: &str) -> Result<(), String> {
  let (base, extension) = name.split_once('.').unwrap_or((name, ""));

  if base.is_empty() {
    return Err("file name is empty".to_owned());
  }

  if base.len() > LEVEL1_NAME {
    return Err(format!(
      "file name '{base}' exceeds {LEVEL1_NAME} characters"
    ));
  }

  if !base
    .bytes()
    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
  {
    return Err(format!(
      "file name '{base}' may contain only A-Z, 0-9 and _"
    ));
  }

  if extension.len() > LEVEL1_EXTENSION {
    return Err(format!(
      "file extension '{extension}' exceeds {LEVEL1_EXTENSION} characters"
    ));
  }

  if !extension
    .bytes()
    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
  {
    return Err(format!(
      "file extension '{extension}' may contain only A-Z, 0-9 and _"
    ));
  }

  Ok(())
}
