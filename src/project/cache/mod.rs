use log::*;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::create_dir_all;
use std::path::PathBuf;

use crate::errors::app_error::AppError;
use crate::hash::sha256_bytes;
use crate::virtual_path::VirtualPath;

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

  /// Forgets `paths` from the index, persists it, and only then deletes the
  /// blobs no remaining entry references.
  ///
  /// Blobs are content-addressed and may be shared with other virtual paths, so
  /// a blob another entry still names is left untouched. Removing several entries
  /// in one call handles that too: their shared blob is deleted once, and only
  /// after none of them reference it.
  ///
  /// The index is saved before any blob is touched. A deletion that fails can
  /// therefore only leave an orphaned blob behind, never an index entry pointing
  /// at content that is gone. Blob deletion is best-effort: the logical removal
  /// has already succeeded and an orphan is preferable to a dangling reference.
  ///
  /// If the index cannot be saved, the removed entries are put back and no blob
  /// is touched, leaving the cache as it was.
  ///
  /// Paths that are not indexed are already in the state the caller asked for.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if the updated index cannot be saved.
  pub fn remove_entries(&mut self, paths: &[VirtualPath]) -> Result<(), AppError> {
    let mut removed = Vec::new();
    for path in paths {
      let key = path.to_string();
      if let Some(hash) = self.index.entries.remove(&key) {
        removed.push((key, hash));
      }
    }

    if removed.is_empty() {
      return Ok(());
    }

    if let Err(err) = self.save() {
      for (key, hash) in &removed {
        self.index.entries.insert(key.clone(), hash.clone());
      }
      return Err(err);
    }

    let hashes: BTreeSet<&str> = removed.iter().map(|(_, hash)| hash.as_str()).collect();

    for hash in hashes {
      if self.index.entries.values().any(|current| current == hash) {
        continue;
      }

      let blob = self.root.join(hash);
      match std::fs::remove_file(&blob) {
        Ok(()) => debug!("Deleted unreferenced cache blob '{}'", blob.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
          trace!("Cache blob '{}' was already gone", blob.display());
        }
        Err(err) => {
          warn!(
            "Could not delete unreferenced cache blob '{}': {err}",
            blob.display()
          );
        }
      }
    }

    Ok(())
  }

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
  fn keeps_a_blob_another_entry_still_references() {
    let entries: [(&str, &[u8]); 2] = [
      ("game.sfc/head.bin", b"HELL"),
      ("other.sfc/head.bin", b"HELL"),
    ];
    let (temp, mut cache) = cache_with(&entries);
    let removed = vpath("game.sfc/head.bin");
    let kept = vpath("other.sfc/head.bin");
    let blob = cache.get_entry(&kept).unwrap();

    cache
      .remove_entries(std::slice::from_ref(&removed))
      .unwrap();

    // Only the entry goes: the blob is still the content of the other path.
    assert_eq!(listed(&cache), [kept.to_string().as_str()]);
    assert!(blob.is_file());
    assert_eq!(std::fs::read(&blob).unwrap(), b"HELL");
    assert_eq!(cache.get_entry(&kept), Some(blob.clone()));

    let reloaded = Cache::new(temp.path().to_path_buf()).unwrap();
    assert_eq!(listed(&reloaded), [kept.to_string().as_str()]);
    assert_eq!(reloaded.get_entry(&kept), Some(blob));
  }

  #[test]
  fn deletes_the_blob_once_no_entry_references_it() {
    let (temp, mut cache) = cache_with(&ENTRIES);
    let target = vpath("game.sfc/rom_bank_00.bin");
    let blob = cache.get_entry(&target).unwrap();

    cache.remove_entries(std::slice::from_ref(&target)).unwrap();

    // The last entry naming the content is gone, so the content is too.
    assert!(!listed(&cache).contains(&target.to_string().as_str()));
    assert!(!blob.exists());

    // The other entries and their blobs are untouched.
    let reloaded = Cache::new(temp.path().to_path_buf()).unwrap();
    assert_eq!(
      listed(&reloaded),
      [
        "game.sfc/rom_bank_00.bin/TITLE.txt",
        "other.sfc/head.bin",
      ]
    );
    for path in listed(&reloaded) {
      assert!(reloaded.get_entry(&vpath(path)).unwrap().is_file());
    }
  }

  #[test]
  fn removing_an_entry_that_is_not_indexed_does_nothing() {
    let (temp, mut cache) = cache_with(&ENTRIES);
    let before = snapshot(temp.path());

    cache
      .remove_entries(&[vpath(
        "game.sfc/never-indexed.bin",
      )])
      .unwrap();

    // A path with no entry names no blob, so nothing is touched at all.
    assert_eq!(snapshot(temp.path()), before);
  }

  #[test]
  fn removing_the_same_entry_twice_is_idempotent() {
    let (temp, mut cache) = cache_with(&ENTRIES);
    let target = vpath("game.sfc/rom_bank_00.bin");

    cache.remove_entries(std::slice::from_ref(&target)).unwrap();
    let after_first = snapshot(temp.path());

    cache.remove_entries(std::slice::from_ref(&target)).unwrap();

    // The second removal has no entry to find and no blob to delete.
    assert_eq!(snapshot(temp.path()), after_first);
    assert!(!after_first.contains_key(&sha256_bytes(b"\x00\x01\x02\x03")));
  }

  #[test]
  fn deletes_a_shared_blob_after_every_entry_naming_it_is_removed() {
    let entries: [(&str, &[u8]); 2] = [
      ("game.sfc/head.bin", b"HELL"),
      ("other.sfc/head.bin", b"HELL"),
    ];
    let (temp, mut cache) = cache_with(&entries);
    let targets = [
      vpath("game.sfc/head.bin"),
      vpath("other.sfc/head.bin"),
    ];
    let blob = cache.get_entry(&targets[0]).unwrap();

    assert_eq!(blob, cache.get_entry(&targets[1]).unwrap());

    cache.remove_entries(&targets).unwrap();

    // Both references went in one call, so the shared blob is deleted once.
    assert!(listed(&cache).is_empty());
    assert!(!blob.exists());

    let reloaded = Cache::new(temp.path().to_path_buf()).unwrap();
    assert!(listed(&reloaded).is_empty());
  }

  #[test]
  fn leaves_an_orphan_when_the_blob_cannot_be_deleted() {
    let (temp, mut cache) = cache_with(&ENTRIES);
    let target = vpath("game.sfc/rom_bank_00.bin");
    let blob = cache.get_entry(&target).unwrap();

    // A directory where the blob belongs cannot be removed as a file.
    std::fs::remove_file(&blob).unwrap();
    std::fs::create_dir(&blob).unwrap();

    // The logical removal has already succeeded, so physical cleanup stays
    // best-effort and the orphan is allowed to remain.
    cache.remove_entries(std::slice::from_ref(&target)).unwrap();

    assert_eq!(cache.get_entry(&target), None);
    assert!(blob.is_dir());

    let reloaded = Cache::new(temp.path().to_path_buf()).unwrap();
    assert_eq!(reloaded.get_entry(&target), None);
    assert!(blob.is_dir());
  }

  #[test]
  fn keeps_every_blob_when_the_index_cannot_be_saved() {
    let (temp, mut cache) = cache_with(&ENTRIES);
    let target = vpath("game.sfc/rom_bank_00.bin");
    let blob = cache.get_entry(&target).unwrap();
    let before: Vec<String> = cache.entries().map(str::to_owned).collect();

    // A directory where the index belongs cannot be written as a file.
    let index = temp.path().join("index.json");
    std::fs::remove_file(&index).unwrap();
    std::fs::create_dir(&index).unwrap();

    match cache
      .remove_entries(std::slice::from_ref(&target))
      .unwrap_err()
    {
      AppError::CacheIndexIo(_) => {}
      err => panic!("Unexpected error: {err:?}"),
    }

    // The persisted index was not updated, the blob is still there, and the
    // in-memory index was rolled back to match it.
    assert!(blob.is_file());
    let after: Vec<String> = cache.entries().map(str::to_owned).collect();
    assert_eq!(after, before);
    assert_eq!(cache.get_entry(&target), Some(blob));
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
