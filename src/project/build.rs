use log::*;
use std::path::{Path, PathBuf};

use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::errors::app_error::AppError;
use crate::manifest::decode_node::DecodeNode;
use crate::manifest::errors::ManifestError;
use crate::project::cache::errors::CacheError;
use crate::virtual_path::VirtualPath;

use super::Project;

/// The business layer of the build.
///
/// A build is the other half of a decode: the manifest records how a source was
/// cut apart, and rebuilding walks those records to put it back together. The
/// source is read from disk, every decode recorded on it is replayed against the
/// bytes the decode before it produced, and the last of them leaves the source
/// again.
///
/// Nothing is written. The decode cache stays canonical, the workspace is only
/// read, and the manifest is never touched: the bytes come back to the caller,
/// who is the one deciding what becomes of them.
///
/// The bytes of an output are resolved in this order:
///
/// 1. an output other decodes are hanging off is rebuilt from those decodes;
/// 2. an output its producing decode node declares an override is read from the
///    workspace;
/// 3. anything else is read from the decode cache.
///
/// An override therefore applies to leaves: `override add` refuses an artifact
/// decodes hang off, so a manifest declaring one over such an artifact was edited
/// by hand, and the decodes below it are what that output is rebuilt from.
///
/// The manifest alone decides which overrides are active. A file sitting in the
/// workspace for an artifact nobody declared an override for is not a thing the
/// build can see, so lifting a declaration disables the override while leaving
/// that file exactly where it is. An override that is declared but whose file is
/// missing or unreadable fails the build rather than quietly falling back to the
/// canonical bytes.
impl Project {
  /// Rebuilds `source` and returns its bytes.
  ///
  /// `source` must be a source the manifest registers: the file it names is what
  /// the decodes recorded against it are replayed over. A source nothing was
  /// ever decoded from is returned as it stands.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if:
  /// - `source` is not a source the manifest registers;
  /// - the source file cannot be read;
  /// - a decode node records a codec the registry does not hold;
  /// - the decodes on one artifact combine in a way the manifest could never
  ///   have recorded: a rebuild codec sharing an artifact with anything else;
  /// - an output recorded by a decode node is no virtual path, is missing from
  ///   the decode cache, or its blob cannot be read;
  /// - an output declared an override has no readable workspace file;
  /// - a codec refuses the artifacts or the parent buffer it is handed.
  pub(crate) fn build(&self, source: &VirtualPath) -> Result<Vec<u8>, AppError> {
    debug!("Building '{}'", source);

    let name = source.to_string();
    let entry = self
      .manifest
      .sources
      .get(Path::new(&name))
      .ok_or_else(|| AppError::SourceNotFound(PathBuf::from(&name)))?;

    let path = self.source_path(Path::new(&name));
    trace!("Reading source '{}'", path.display());

    let bytes = std::fs::read(&path).map_err(|err| AppError::SourceFileIo(path.clone(), err))?;

    entry
      .sha256_matches(&bytes)
      .map_err(|actual| AppError::SourceFileMismatch(path.clone(), entry.sha256.clone(), actual))?;

    let decodes = self
      .manifest
      .decodes
      .get(&name)
      .map(Vec::as_slice)
      .unwrap_or(&[]);

    self.rebuild(source, bytes, decodes)
  }

  /// Applies every decode recorded on `path` to `bytes`, in manifest order.
  ///
  /// `bytes` are what the artifact holds at this point: the source file for a
  /// source, its canonical bytes for an artifact, and whatever the last decode
  /// produced for anything deeper than that. Each decode is handed what the one
  /// before it returned, which is what keeps sibling decodes from losing each
  /// other's changes: patches compose over the accumulated bytes, and a rebuild
  /// constructs the whole artifact and therefore has it to itself.
  ///
  /// The kind of the first decode on an artifact decides what may follow it. A
  /// patch is one of several and composes with the rest; a rebuild codec owns the
  /// artifact, so nothing may share it. [`Manifest::add_decode`] already refuses
  /// to record anything else, so a manifest that says otherwise was edited by
  /// hand, and is refused here rather than composed into a result no decode ever
  /// described.
  ///
  /// [`Manifest::add_decode`]: crate::manifest::Manifest::add_decode
  fn rebuild(
    &self,
    path: &VirtualPath,
    mut bytes: Vec<u8>,
    decodes: &[DecodeNode],
  ) -> Result<Vec<u8>, AppError> {
    let mut kind: Option<CodecKind> = None;

    for node in decodes {
      let codec = self
        .codecs
        .get(&node.codec.id)
        .ok_or_else(|| AppError::CodecUnavailable(node.codec.id.clone()))?;

      match (kind, codec.kind()) {
        (None, _) | (Some(CodecKind::Patch), CodecKind::Patch) => {}
        _ => {
          return Err(
            ManifestError::ConflictingDecodeKind(node.name.clone(), path.to_string()).into(),
          );
        }
      }
      kind = Some(codec.kind());

      debug!("Encoding '{}' ({}) on '{path}'", node.name, node.codec.id);

      let args = node.codec_args();
      let artifacts = self.resolve_outputs(path, node)?;

      bytes = codec.encode(bytes, &artifacts, args)?;
    }

    Ok(bytes)
  }

  /// Resolves the bytes of every output `node` recorded, in the order it recorded
  /// them.
  ///
  /// The order is the codec's own, and `encode` is handed the artifacts exactly
  /// as the manifest lists them: nothing here sorts, deduplicates or repairs it.
  /// An output name that no virtual path can represent is refused rather than
  /// skipped, since a decode whose outputs cannot be addressed cannot be encoded
  /// from them.
  fn resolve_outputs(
    &self,
    path: &VirtualPath,
    node: &DecodeNode,
  ) -> Result<Vec<DecodedArtifact>, AppError> {
    node
      .outputs
      .iter()
      .map(|output| {
        Ok(DecodedArtifact {
          name: output.to_owned(),
          data: self.resolve_output(path, node, output)?,
        })
      })
      .collect()
  }

  /// Resolves the bytes of a single output of `node`.
  ///
  /// An artifact other decodes hang off is rebuilt from those decodes, over its
  /// own canonical bytes: the decodes below it are what turns it into what this
  /// node is about to encode it into. An artifact nothing decodes is a leaf, and
  /// a leaf is where an override lives.
  fn resolve_output(
    &self,
    path: &VirtualPath,
    node: &DecodeNode,
    output: &str,
  ) -> Result<Vec<u8>, AppError> {
    let output_path = path.join(output)?;

    // A decode node declaring no decodes still has an empty bucket once
    // `decode add` has been pointed at it, so whether the artifact has decodes of
    // its own is the question, not whether it has a bucket.
    if !node.is_terminal_output(output) {
      trace!("'{output_path}' is rebuilt from the decodes hanging off it");

      return self.rebuild(
        &output_path,
        self.canonical(&output_path)?,
        node.decodes.get(output).map(Vec::as_slice).unwrap_or(&[]),
      );
    }

    // Only the declaration in the manifest activates an override. A workspace
    // file left behind by a removed override, or dropped there by hand, is never
    // read by a build.
    if node.has_override_for(output) {
      let workspace = self.workspace_path(&output_path);
      trace!(
        "Reading overridden '{output_path}' from '{}'",
        workspace.display()
      );

      return std::fs::read(&workspace)
        .map_err(|err| AppError::WorkspaceFileReadIo(workspace, err));
    }

    trace!("Reading canonical bytes of '{output_path}'");
    self.canonical(&output_path)
  }

  /// Returns the canonical bytes the decode cache holds for `path`.
  ///
  /// The cache is read-only here, and it is the one place the bytes of an
  /// untouched artifact come from. An artifact the index does not name, or one
  /// whose blob has since been removed from under it, is refused rather than
  /// treated as empty: a build that filled the gap would hand back a hole where
  /// the artifact belonged.
  fn canonical(&self, path: &VirtualPath) -> Result<Vec<u8>, AppError> {
    let blob = self
      .cache
      .get_entry(path)
      .ok_or_else(|| CacheError::NotIndexed(path.to_string()))?;

    std::fs::read(&blob).map_err(|err| CacheError::BlobIo(path.to_string(), err).into())
  }
}

#[cfg(test)]
mod tests {
  use std::collections::BTreeMap;

  use crate::codecs::cdrom::iso9660::Iso9660;
  use crate::codecs::errors::CodecError;
  use crate::codecs::nintendo::snes::cart_lorom::SnesLoRom;
  use crate::codecs::nintendo::snes::common::LOROM_HEADER_OFFSET;
  use crate::codecs::nintendo::snes::common::testkit;
  use crate::testing::fixtures::lorom;
  use crate::testing::fixtures::project::ProjectFixture;
  use crate::virtual_path::errors::VirtualPathError;

  use super::*;

  const EXTRACT: &str = "std/generic/extract";
  const ISO9660: &str = "std/cdrom/iso9660";
  const LOROM: &str = "std/nintendo/snes/cart/lorom";

  const SOURCE: &[u8] = b"HELLOWORLD";

  const RETITLE: &[u8] = b"Smashing The Stack For Ludared Rules!";

  fn vpath(path: &str) -> VirtualPath {
    VirtualPath::new(path).unwrap()
  }

  fn of(source: &str, artifact: &str) -> VirtualPath {
    vpath(&format!("{source}/{artifact}"))
  }

  fn manifest(source: &str, decodes: &[String], bytes: &[u8]) -> String {
    format!(
      r#"{{
  "sources": {{ "{source}": {{ "sha256": "{}", "size": null, "label": null }} }},
  "decodes": {{ "{source}": [ {} ] }}
}}"#,
      ProjectFixture::sha256_hex(bytes),
      decodes
        .iter()
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .join(", ")
    )
  }

  fn extract(name: &str, offset: usize, length: usize) -> String {
    // The manifest records arguments as the string the codec is handed, so the
    // JSON is written there as a JSON string rather than as a nested object.
    let args = serde_json::json!({ "name": name, "offset": offset, "length": length }).to_string();
    let args = serde_json::Value::String(args).to_string();

    format!(
      r#"{{
      "name": "{name}",
      "codec": {{
        "id": "{EXTRACT}",
        "version": 1,
        "args": {args}
      }},
      "outputs": [ "{name}" ],
      "decodes": {{}}
    }}"#
    )
  }

  fn head() -> String {
    extract("head.bin", 0, 5)
  }

  fn banks() -> String {
    format!(
      r#"{{
      "name": "rom_banks",
      "codec": {{ "id": "{LOROM}", "version": 1, "args": null }},
      "outputs": [ "rom_bank_00.bin", "rom_bank_01.bin" ],
      "decodes": {{}}
    }}"#
    )
  }

  fn unpacked(source_bytes: &[u8], decodes: &[String]) -> (ProjectFixture, String) {
    let (source, path) = ProjectFixture::random_source_name();
    let mut fixture = ProjectFixture::new();

    fixture.register_source_file(&path, source_bytes);
    fixture.write_manifest(manifest(&source, decodes, source_bytes));
    fixture.reload();

    fixture.project.unpack().unwrap();

    (fixture, source)
  }

  fn unpacked_lorom() -> (ProjectFixture, String) {
    let (mut fixture, source) = lorom::project();

    fixture.project.unpack().unwrap();

    (fixture, source)
  }

  fn redecodes(fixture: &mut ProjectFixture, source: &str, bytes: &[u8], decodes: &[String]) {
    fixture.write_manifest(manifest(source, decodes, bytes));
    fixture.reload();
  }

  fn edit(fixture: &mut ProjectFixture, artifact: &VirtualPath, bytes: &[u8]) {
    fixture.project.add_override(artifact, false).unwrap();
    std::fs::write(fixture.project.workspace_path(artifact), bytes).unwrap();
  }

  fn title_offset() -> usize {
    lorom::SOURCE
      .windows(lorom::TITLE.len())
      .position(|window| window == lorom::TITLE)
      .expect("the fixture should embed the article its decode node names")
  }

  fn cache_of(fixture: &ProjectFixture) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(fixture.project.cache.root.clone())
      .unwrap()
      .map(|entry| {
        let entry = entry.unwrap();
        (
          entry.file_name().to_string_lossy().into_owned(),
          std::fs::read(entry.path()).unwrap(),
        )
      })
      .collect()
  }

  fn written(fixture: &ProjectFixture) -> String {
    std::fs::read_to_string(
      fixture
        .project
        .root
        .join(&fixture.project.configuration.project.manifest),
    )
    .unwrap()
  }

  fn build_err(project: &Project, source: &str) -> AppError {
    project.build(&vpath(source)).unwrap_err()
  }

  #[test]
  fn returns_the_bytes_of_a_source_nothing_was_decoded_from() {
    let (fixture, source) = unpacked(SOURCE, &[]);

    assert_eq!(fixture.project.build(&vpath(&source)).unwrap(), SOURCE);
  }

  #[test]
  fn refuses_a_path_the_manifest_does_not_register_as_a_source() {
    let decodes = [head()];
    let (fixture, source) = unpacked(SOURCE, &decodes);

    for path in [
      "elsewhere.sfc".to_owned(),
      of(&source, "head.bin").to_string(),
      format!("{source}/missing.bin"),
    ] {
      match build_err(&fixture.project, &path) {
        AppError::SourceNotFound(reported) => assert_eq!(reported, PathBuf::from(&path)),
        err => panic!("'{path}' reported: {err:?}"),
      }
    }
  }

  #[test]
  fn fails_when_the_source_file_cannot_be_read() {
    let (fixture, source) = unpacked(SOURCE, &[]);

    std::fs::remove_file(fixture.project.source_path(Path::new(&source))).unwrap();

    assert!(matches!(
      build_err(&fixture.project, &source),
      AppError::SourceFileIo(_, _)
    ));
  }

  #[test]
  fn reproduces_a_valid_source_byte_for_byte() {
    let (fixture, source) = unpacked_lorom();

    assert_eq!(
      fixture.project.build(&vpath(&source)).unwrap(),
      lorom::SOURCE
    );
  }

  #[test]
  fn rebuilds_an_overridden_leaf_all_the_way_up_to_the_source() {
    let (mut fixture, source) = unpacked_lorom();
    let title = of(&source, lorom::TITLE_ARTIFACT);

    edit(&mut fixture, &title, RETITLE);

    let rebuilt = fixture.project.build(&vpath(&source)).unwrap();
    let offset = title_offset();

    assert_eq!(rebuilt.len(), lorom::SOURCE.len());
    assert_eq!(
      &rebuilt[offset..offset + RETITLE.len()],
      RETITLE,
      "the rebuilt image should carry the edited article"
    );

    assert_ne!(
      testkit::fields(&rebuilt, LOROM_HEADER_OFFSET),
      testkit::fields(lorom::SOURCE, LOROM_HEADER_OFFSET),
      "the checksum should have followed the change"
    );
    testkit::assert_valid_header(&SnesLoRom, &rebuilt, LOROM_HEADER_OFFSET);

    let moved: i32 = RETITLE
      .iter()
      .zip(lorom::TITLE)
      .map(|(after, before)| *after as i32 - *before as i32)
      .sum();

    assert_eq!(
      testkit::fields(&rebuilt, LOROM_HEADER_OFFSET).1,
      testkit::fields(lorom::SOURCE, LOROM_HEADER_OFFSET)
        .1
        .wrapping_add(moved as u16)
    );
  }

  #[test]
  fn ignores_a_workspace_file_the_manifest_does_not_declare() {
    let (fixture, source) = unpacked_lorom();

    let workspace = fixture
      .project
      .workspace_path(&of(&source, lorom::TITLE_ARTIFACT));
    std::fs::create_dir_all(workspace.parent().unwrap()).unwrap();
    std::fs::write(&workspace, RETITLE).unwrap();

    assert_eq!(
      fixture.project.build(&vpath(&source)).unwrap(),
      lorom::SOURCE
    );
  }

  #[test]
  fn stops_reading_the_workspace_once_the_declaration_is_removed() {
    let (mut fixture, source) = unpacked_lorom();
    let title = of(&source, lorom::TITLE_ARTIFACT);

    edit(&mut fixture, &title, RETITLE);
    fixture.project.remove_override(&title).unwrap();

    assert!(fixture.project.workspace_path(&title).is_file());
    assert_eq!(
      fixture.project.build(&vpath(&source)).unwrap(),
      lorom::SOURCE
    );
  }

  #[test]
  fn leaves_the_decode_cache_canonical_while_an_override_is_active() {
    let (mut fixture, source) = unpacked_lorom();
    let title = of(&source, lorom::TITLE_ARTIFACT);

    edit(&mut fixture, &title, RETITLE);
    fixture.project.build(&vpath(&source)).unwrap();

    let blob = fixture.project.cache.get_entry(&title).unwrap();

    assert_eq!(std::fs::read(blob).unwrap(), lorom::TITLE);
  }

  #[test]
  fn writes_nothing_while_building() {
    let (mut fixture, source) = unpacked_lorom();
    let title = of(&source, lorom::TITLE_ARTIFACT);

    edit(&mut fixture, &title, RETITLE);
    let cache = cache_of(&fixture);
    let manifest = written(&fixture);

    fixture.project.build(&vpath(&source)).unwrap();

    assert_eq!(cache_of(&fixture), cache);
    assert_eq!(written(&fixture), manifest);
  }

  #[test]
  fn fails_when_an_active_override_has_no_readable_workspace_file() {
    let (mut fixture, source) = unpacked_lorom();
    let title = of(&source, lorom::TITLE_ARTIFACT);

    edit(&mut fixture, &title, RETITLE);

    std::fs::remove_file(fixture.project.workspace_path(&title)).unwrap();

    match build_err(&fixture.project, &source) {
      AppError::WorkspaceFileReadIo(path, _) => assert!(
        path.ends_with(lorom::TITLE_ARTIFACT),
        "the error should name the workspace file: {}",
        path.display()
      ),
      err => panic!("Unexpected error: {err:?}"),
    }

    std::fs::create_dir_all(fixture.project.workspace_path(&title)).unwrap();

    assert!(matches!(
      build_err(&fixture.project, &source),
      AppError::WorkspaceFileReadIo(_, _)
    ));
  }

  #[test]
  fn applies_sibling_patches_over_one_another() {
    let (mut fixture, source) = unpacked(
      SOURCE,
      &[
        extract("head.bin", 0, 5),
        extract("tail.bin", 5, 5),
      ],
    );

    edit(&mut fixture, &of(&source, "head.bin"), b"HEAD!");
    edit(&mut fixture, &of(&source, "tail.bin"), b"TAIL!");

    assert_eq!(
      fixture.project.build(&vpath(&source)).unwrap(),
      b"HEAD!TAIL!"
    );
  }

  #[test]
  fn applies_overlapping_patches_in_the_order_the_manifest_records_them() {
    for (decodes, expected) in [
      (
        vec![
          extract("head.bin", 0, 5),
          extract("tail.bin", 2, 5),
        ],
        &b"AABBBBBRLD"[..],
      ),
      (
        vec![
          extract("tail.bin", 2, 5),
          extract("head.bin", 0, 5),
        ],
        &b"AAAAABBRLD"[..],
      ),
    ] {
      let (mut fixture, source) = unpacked(SOURCE, &decodes);

      edit(&mut fixture, &of(&source, "head.bin"), b"AAAAA");
      edit(&mut fixture, &of(&source, "tail.bin"), b"BBBBB");

      assert_eq!(
        fixture.project.build(&vpath(&source)).unwrap(),
        expected,
        "the patches should compose in the order they are recorded in"
      );
    }
  }

  #[test]
  fn fails_when_a_canonical_artifact_is_missing_from_the_cache() {
    let (fixture, source) = lorom::project();

    match build_err(&fixture.project, &source) {
      AppError::CacheError(CacheError::NotIndexed(path)) => assert_eq!(
        path,
        of(&source, "rom_bank_00.bin").to_string(),
        "the first artifact the build needed should be the one reported"
      ),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn fails_when_a_cached_blob_is_gone() {
    let (fixture, source) = unpacked_lorom();

    let blob = fixture
      .project
      .cache
      .get_entry(&of(&source, "rom_bank_00.bin"))
      .unwrap();
    std::fs::remove_file(&blob).unwrap();

    match build_err(&fixture.project, &source) {
      AppError::CacheError(CacheError::BlobIo(path, _)) => {
        assert_eq!(path, of(&source, "rom_bank_00.bin").to_string())
      }
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn fails_on_an_output_that_is_no_virtual_path() {
    let decodes = [head()];
    let (mut fixture, source) = unpacked(SOURCE, &decodes);

    let broken = [extract(
      "data//head.bin",
      0,
      5,
    )];
    redecodes(&mut fixture, &source, SOURCE, &broken);

    match build_err(&fixture.project, &source) {
      AppError::VirtualPathError(VirtualPathError::EmptyComponent(path)) => assert_eq!(
        path,
        format!("{source}/data//head.bin"),
        "the offending output should be named"
      ),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn resolves_every_output_a_decode_node_records() {
    let decodes = [head()];
    let (mut fixture, source) = unpacked(SOURCE, &decodes);

    let recorded = [decodes[0].replace(
      r#""outputs": [ "head.bin" ]"#,
      r#""outputs": [ "head.bin", "tail.bin" ]"#,
    )];

    redecodes(&mut fixture, &source, SOURCE, &recorded);

    match build_err(&fixture.project, &source) {
      AppError::CacheError(CacheError::NotIndexed(path)) => {
        assert_eq!(path, of(&source, "tail.bin").to_string())
      }
      err => panic!("Unexpected error: {err:?}"),
    }

    fixture
      .project
      .cache
      .add_entry(&of(&source, "tail.bin"), b"WORLD")
      .unwrap();

    match build_err(&fixture.project, &source) {
      AppError::CodecError(CodecError::Message(message)) => assert!(
        message.contains("Expected exactly one artifact"),
        "unexpected message: {message}"
      ),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn fails_when_a_codec_refuses_what_it_is_handed() {
    let decodes = [head()];
    let (mut fixture, source) = unpacked(SOURCE, &decodes);

    edit(&mut fixture, &of(&source, "head.bin"), b"TOO LONG");

    match build_err(&fixture.project, &source) {
      AppError::CodecError(CodecError::Message(message)) => assert!(
        message.contains("Expected 5 bytes for artifact 'head.bin'"),
        "unexpected message: {message}"
      ),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn fails_on_a_codec_the_registry_does_not_hold() {
    let decodes = [head()];
    let (mut fixture, source) = unpacked(SOURCE, &decodes);

    redecodes(
      &mut fixture,
      &source,
      SOURCE,
      &[r#"{
      "name": "gone",
      "codec": { "id": "std/gone/missing", "version": 1, "args": null },
      "outputs": [ "gone.bin" ],
      "decodes": {}
    }"#
        .to_owned()],
    );

    match build_err(&fixture.project, &source) {
      AppError::CodecUnavailable(id) => assert_eq!(id, "std/gone/missing"),
      err => panic!("Unexpected error: {err:?}"),
    }
  }

  #[test]
  fn refuses_decodes_the_manifest_could_never_have_recorded() {
    for (decodes, conflicting) in [
      (vec![banks(), banks()], "rom_banks"),
      (
        vec![
          banks(),
          extract("patch.bin", 0, 4),
        ],
        "patch.bin",
      ),
      (
        vec![
          extract("patch.bin", 0, 4),
          banks(),
        ],
        "rom_banks",
      ),
    ] {
      let (fixture, source) = unpacked(lorom::SOURCE, &decodes);

      match build_err(&fixture.project, &source) {
        AppError::ManifestError(ManifestError::ConflictingDecodeKind(name, under)) => {
          assert_eq!(name, conflicting);
          assert_eq!(under, source);
        }
        err => panic!("Unexpected error for {decodes:?}: {err:?}"),
      }
    }
  }

  #[test]
  fn rejects_source_bytes_mismatch_manifest() {
    let (fixture, source) = unpacked(SOURCE, &[]);

    // Modify the source file on disk so its bytes differ from the manifest sha256.
    let sources_dir = fixture
      .project
      .root
      .join(&fixture.project.configuration.paths.sources);
    let source_path = sources_dir.join(&source);
    let original = std::fs::read(&source_path).unwrap();
    let mutated = [
      &original[..original.len().saturating_sub(1)],
      &[0u8],
    ]
    .concat();
    std::fs::write(&source_path, &mutated).unwrap();

    match build_err(&fixture.project, &source) {
      AppError::SourceFileMismatch(..) => {}
      err => panic!("Expected SourceFileMismatch, got {err:?}"),
    }
  }

  /// An ISO 9660 image recorded the way the core records any other source, and
  /// rebuilt from the artifacts its decode produced.
  ///
  /// The unit tests beside the codec cover it in isolation; this one covers the
  /// part only the core can show, that the order a decode produced its
  /// artifacts in is the order the artifacts are handed back in, which is what
  /// the table of contents leading them relies on.
  fn iso_project() -> (ProjectFixture, String) {
    let image = iso_fixture();
    let source = "game.iso".to_owned();

    let mut fixture = ProjectFixture::new();
    fixture.register_source_file(&source, &image);

    fixture
      .project
      .add_decode(
        &vpath(&source),
        ISO9660.to_owned(),
        &None,
        "iso".to_owned(),
        iso_decode(&image),
      )
      .unwrap();

    fixture.project.unpack().unwrap();

    (fixture, source)
  }

  /// An ISO holding two files and a directory, written through the same writer
  /// the codec rebuilds with.
  fn iso_fixture() -> Vec<u8> {
    use hadris_iso::write::options::CreationFeatures;
    use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
    use std::sync::Arc;

    let mut buffer = std::io::Cursor::new(Vec::new());
    IsoImageWriter::create(
      &mut buffer,
      InputFiles {
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
        files: vec![
          IsoFile::File {
            name: Arc::new("Z.BIN".to_owned()),
            contents: b"z".to_vec(),
          },
          IsoFile::Directory {
            name: Arc::new("A".to_owned()),
            children: vec![IsoFile::File {
              name: Arc::new("FIRST.BIN".to_owned()),
              contents: b"first".to_vec(),
            }],
          },
        ],
      },
      hadris_iso::write::options::IsoFormatOptions {
        volume_name: "TEST".to_owned(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
        features: CreationFeatures::rock_ridge(),
        strict_charset: false,
      },
    )
    .unwrap();

    buffer.into_inner()
  }

  fn iso_decode(image: &[u8]) -> Vec<DecodedArtifact> {
    crate::codecs::Codec::decode(&Iso9660, image, None).unwrap()
  }

  /// The files the image carries, read back through Hadris.
  fn iso_files(image: &[u8]) -> Vec<(String, Vec<u8>)> {
    use hadris_iso::read::IsoImage;

    let image = IsoImage::open(std::io::Cursor::new(image.to_vec())).unwrap();
    let root = image.root_dir();

    let mut found = Vec::new();
    let mut pending = vec![(root.iter(&image), String::new())];

    while let Some((directory, prefix)) = pending.pop() {
      for entry in directory.entries() {
        let entry = entry.unwrap();
        if entry.is_special() {
          continue;
        }

        let raw = entry.record.name();
        let raw = match raw.iter().position(|byte| *byte == b';') {
          Some(index) => &raw[..index],
          None => raw,
        };
        let name = String::from_utf8_lossy(raw).into_owned();
        let path = if prefix.is_empty() {
          name.clone()
        } else {
          format!("{prefix}/{name}")
        };

        if entry.is_directory() {
          pending.push((image.open_dir(entry.as_dir_ref(&image).unwrap()), path));
          continue;
        }

        found.push((path, image.read_file(&entry).unwrap()));
      }
    }

    found.sort();
    found
  }

  #[test]
  fn rebuilds_an_iso_9660_image_from_a_recorded_decode() {
    let (fixture, source) = iso_project();
    let image = iso_fixture();

    let rebuilt = fixture.project.build(&vpath(&source)).unwrap();

    assert_eq!(
      iso_files(&rebuilt),
      iso_files(&image),
      "the rebuilt image should hold the source's files"
    );
  }

  #[test]
  fn rebuilds_an_iso_9660_image_carrying_an_edited_file() {
    let (mut fixture, source) = iso_project();
    let edited = b"an edited file, and a good deal longer than it was";

    edit(&mut fixture, &of(&source, "Z.BIN"), edited);

    let rebuilt = fixture.project.build(&vpath(&source)).unwrap();
    let files = iso_files(&rebuilt);

    assert_eq!(
      files,
      [
        ("A/FIRST.BIN".to_owned(), b"first".to_vec()),
        ("Z.BIN".to_owned(), edited.to_vec()),
      ],
      "the edit should have reached the rebuilt image"
    );
  }

  #[test]
  fn records_the_metadata_before_the_files() {
    let (fixture, source) = iso_project();

    // What the rebuild relies on is the metadata leading the artifacts the
    // manifest recorded, which is what `decode` returning it first buys.
    let outputs = &fixture.project.manifest.decodes[&source][0].outputs;

    assert_eq!(
      outputs[0], "ISO9660.json",
      "the metadata should be recorded first"
    );
    assert_eq!(
      &outputs[1..],
      [
        "Z.BIN",
        "A/FIRST.BIN"
      ]
    );
  }
}
