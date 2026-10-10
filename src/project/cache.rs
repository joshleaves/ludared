use crate::errors::app_error::AppError;
use crate::hash::sha256_bytes;
use crate::virtual_path::VirtualPath;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::create_dir_all;
use std::path::PathBuf;

pub(crate) mod errors;

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct CacheIndex {
  entries: BTreeMap<String, String>,
}

#[derive(Debug)]
pub(crate) struct Cache {
  pub(crate) root: PathBuf,
  pub(crate) index: CacheIndex,
}

impl Cache {
  pub fn new(root: PathBuf) -> Result<Self, AppError> {
    create_dir_all(&root)?;
    let index = match std::fs::read(root.join("index.json")) {
      Ok(data) => serde_json::from_slice(&data).map_err(AppError::CacheIndexJson)?,
      Err(err) => match err.kind() {
        std::io::ErrorKind::NotFound => CacheIndex::default(),
        _ => return Err(err.into()),
      },
    };

    Ok(Self { root, index })
  }

  /// Returns every virtual path currently in the index, in lexicographic order.
  ///
  /// The index is a [`BTreeMap`], so the order is a property of the index rather
  /// than of the order entries happened to be added in, and does not change
  /// between runs. The index is the only thing it consults: blobs on disk are
  /// never scanned.
  pub fn entries(&self) -> impl Iterator<Item = &str> {
    self.index.entries.keys().map(String::as_str)
  }

  /// Forgets every entry in the in-memory index, leaving stored content alone.
  ///
  /// Nothing reaches disk until [`Self::save`], so this only makes sense as the
  /// first half of rebuilding an index from scratch.
  pub fn clear_index(&mut self) {
    self.index = CacheIndex::default();
  }

  pub fn add_entry(&mut self, path: &VirtualPath, bytes: &[u8]) -> Result<(), AppError> {
    let hash = sha256_bytes(bytes);
    std::fs::write(self.root.join(&hash), bytes)?;
    self.index.entries.insert(path.to_string(), hash);
    Ok(())
  }

  pub fn get_entry(&self, path: &VirtualPath) -> Option<PathBuf> {
    Some(self.root.join(self.index.entries.get(&path.to_string())?))
  }

  // pub fn remove_entry(&mut self, path: &VirtualPath) -> Result<(), AppError> {
  //   self.index.entries.remove_entry(&path.to_string());
  //   Ok(())
  // }

  pub fn save(&self) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(&self.index).map_err(AppError::CacheIndexJson)?;
    std::fs::write(self.root.join("index.json"), json).map_err(AppError::CacheIndexIo)?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  use tempfile::TempDir;

  use super::*;
  use crate::testing::fixtures::lorom;

  /// Entries added out of order, so that a listing taken from the index cannot be
  /// confused with one taken in insertion order.
  const ENTRIES: [(&str, &[u8]); 3] = [
    ("game.sfc/rom_bank_00.bin", b"\x00\x01\x02\x03"),
    ("other.sfc/head.bin", b"BYE"),
    ("game.sfc/rom_bank_00.bin/TITLE.txt", b"HEL"),
  ];

  /// A cache rooted in a temporary directory and holding `entries`.
  ///
  /// The temporary directory is returned alongside it because dropping it would
  /// take the blobs with it.
  fn cache_with(entries: &[(&str, &[u8])]) -> (TempDir, Cache) {
    let temp = TempDir::new().unwrap();
    let mut cache = Cache::new(temp.path().to_path_buf()).unwrap();

    for (virtual_path, bytes) in entries {
      cache
        .add_entry(&VirtualPath::new(virtual_path).unwrap(), bytes)
        .unwrap();
    }
    cache.save().unwrap();

    (temp, cache)
  }

  /// Every file under the cache root, keyed by file name, with its contents.
  fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(root)
      .unwrap()
      .map(|entry| {
        let entry = entry.unwrap();
        let bytes = std::fs::read(entry.path()).unwrap();
        (entry.file_name().to_string_lossy().into_owned(), bytes)
      })
      .collect()
  }

  fn listed(cache: &Cache) -> Vec<&str> {
    cache.entries().collect()
  }

  fn vpath(path: &str) -> VirtualPath {
    VirtualPath::new(path).unwrap()
  }

  #[test]
  fn lists_every_indexed_virtual_path_in_lexicographic_order() {
    let (_temp, cache) = cache_with(&ENTRIES);

    assert_eq!(
      listed(&cache),
      [
        "game.sfc/rom_bank_00.bin",
        "game.sfc/rom_bank_00.bin/TITLE.txt",
        "other.sfc/head.bin",
      ]
    );
  }

  #[test]
  fn listing_is_independent_of_the_order_entries_were_added_in() {
    let (_forward, forward) = cache_with(&ENTRIES);
    let (_reverse, reverse) = cache_with(&[
      ENTRIES[2],
      ENTRIES[1],
      ENTRIES[0],
    ]);

    assert_eq!(listed(&forward), listed(&reverse));
  }

  #[test]
  fn listing_does_not_depend_on_the_blobs_present_on_disk() {
    // A blob left behind by an index entry that no longer exists must not show
    // up, and removing a blob the index does name must not hide its entry.
    let (_temp, mut cache) = cache_with(&ENTRIES);
    cache.clear_index();
    cache.save().unwrap();
    assert!(listed(&cache).is_empty());

    let (_temp, cache) = cache_with(&ENTRIES);
    let blob = cache.get_entry(&vpath("game.sfc/rom_bank_00.bin")).unwrap();
    std::fs::remove_file(blob).unwrap();

    assert_eq!(listed(&cache).len(), ENTRIES.len());
  }

  #[test]
  fn names_the_content_addressed_blob_of_an_indexed_virtual_path() {
    let (_temp, cache) = cache_with(&ENTRIES);

    let blob = cache
      .get_entry(&vpath("game.sfc/rom_bank_00.bin/TITLE.txt"))
      .unwrap();

    assert_eq!(blob, cache.root.join(sha256_bytes(b"HEL")));
    assert!(blob.is_file());
  }

  #[test]
  fn the_blob_an_indexed_virtual_path_names_holds_its_bytes() {
    let (temp, _cache) = cache_with(&ENTRIES);
    // Reloaded so that the persisted index, rather than the in-memory one, is
    // what resolves the virtual path.
    let cache = Cache::new(temp.path().to_path_buf()).unwrap();

    for (virtual_path, bytes) in ENTRIES {
      let blob = cache.get_entry(&vpath(virtual_path)).unwrap();

      assert_eq!(std::fs::read(blob).unwrap(), bytes);
    }
  }

  #[test]
  fn virtual_paths_holding_the_same_bytes_name_a_single_blob() {
    let entries: [(&str, &[u8]); 2] = [
      ("game.sfc/head.bin", b"HELL"),
      ("other.sfc/head.bin", b"HELL"),
    ];
    let (_temp, cache) = cache_with(&entries);

    let first = cache.get_entry(&vpath("game.sfc/head.bin")).unwrap();
    let second = cache.get_entry(&vpath("other.sfc/head.bin")).unwrap();

    assert_eq!(first, second);
  }

  #[test]
  fn names_nothing_for_a_virtual_path_that_is_not_indexed() {
    let (_temp, cache) = cache_with(&ENTRIES);

    for virtual_path in [
      // Never cached at all.
      "game.sfc/missing.bin",
      // Cached under a different source.
      "third.sfc/rom_bank_00.bin/TITLE.txt",
      // A prefix of an indexed path is not itself indexed.
      "game.sfc/rom_bank_00.bin/TITLE",
      // A source is a workspace path, and the cache never holds one.
      "game.sfc",
    ] {
      assert_eq!(
        cache.get_entry(&vpath(virtual_path)),
        None,
        "'{virtual_path}' should not resolve to a blob"
      );
    }
  }

  #[test]
  fn reading_a_blob_through_the_cache_leaves_it_untouched() {
    let (temp, cache) = cache_with(&ENTRIES);
    let before = snapshot(temp.path());

    for virtual_path in listed(&cache) {
      assert!(cache.get_entry(&vpath(virtual_path)).unwrap().is_file());
    }

    // Neither the index nor the blobs are rewritten, and nothing is added to or
    // removed from the cache root.
    assert_eq!(snapshot(temp.path()), before);
  }

  #[test]
  fn lists_the_nested_artifacts_a_real_lorom_unpacks_to() {
    let (mut fixture, source) = lorom::project();

    fixture.project.unpack().unwrap();
    fixture.reload();

    assert_eq!(listed(&fixture.project.cache).len(), 3);

    let title = lorom::title_vpath(&source);
    let blob = fixture.project.cache.get_entry(&vpath(&title)).unwrap();

    assert_eq!(std::fs::read(&blob).unwrap(), lorom::TITLE);
    assert_eq!(
      blob,
      fixture.project.cache.root.join(sha256_bytes(lorom::TITLE))
    );
  }
}
