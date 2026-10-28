use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use crate::codecs::Codec;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::errors::CodecError;
use crate::codecs::registry::CodecRegistry;
use crate::errors::app_error::AppError;
use crate::manifest::decode_node::CodecNode;
use crate::project::overrides::errors::OverrideError;
use crate::source::Source;
use crate::virtual_path::VirtualPath;
use decode_node::DecodeNode;
use errors::ManifestError;

pub mod decode_node;
pub mod errors;

#[derive(Clone, Copy)]
pub enum WalkTarget {
  Paths,
  Decodes,
  Overrides,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Manifest {
  #[serde(default)]
  pub sources: HashMap<PathBuf, Source>,
  #[serde(default)]
  pub decodes: HashMap<String, Vec<DecodeNode>>,
}

impl Manifest {
  pub fn load(path: &Path) -> Result<Self, AppError> {
    let content = std::fs::read_to_string(path).map_err(AppError::ManifestFileIo)?;
    // you may want a Json variant later
    let manifest: Self = serde_json::from_str(&content).map_err(AppError::ManifestFileJson)?;
    Ok(manifest)
  }

  pub fn save(&self, path: &Path) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(self).map_err(AppError::ManifestFileJson)?;
    std::fs::write(path, json).map_err(AppError::ManifestFileIo)?;
    Ok(())
  }

  pub fn artifacts(&self) -> Vec<VirtualPath> {
    self.list_paths(WalkTarget::Paths)
  }

  pub fn decodes(&self) -> Vec<VirtualPath> {
    self.list_paths(WalkTarget::Decodes)
  }

  pub fn overrides(&self) -> Vec<VirtualPath> {
    self.list_paths(WalkTarget::Overrides)
  }

  /// Private convenience method: walk through the whole tree
  fn list_paths(&self, target: WalkTarget) -> Vec<VirtualPath> {
    let mut paths = Vec::new();

    for source in self.sources.keys().filter_map(|source| source.to_str()) {
      if matches!(target, WalkTarget::Paths) {
        paths.push(source.to_owned());
      }

      if let Some(decodes) = self.decodes.get(source) {
        for decode in decodes {
          if matches!(target, WalkTarget::Decodes) {
            paths.push(source.to_owned());
          }

          decode.visit_into(source, target, &mut paths);
        }
      }
    }

    paths.sort();
    paths.dedup();

    paths
      .iter()
      .filter_map(|path| VirtualPath::new(path).ok())
      .collect()
  }

  /// Returns the mutable decode bucket attached to the artifact at the given
  /// virtual path, creating the bucket if necessary.
  ///
  /// The target artifact must already exist as either a source or an output of
  /// an existing decode node. This method may create a `Vec<DecodeNode>` for that
  /// artifact, but never creates sources, artifacts, outputs, or intermediate
  /// decode nodes.
  ///
  /// # Returns
  ///
  /// - `Ok(decodes)` when the artifact exists. Its decode bucket is created if
  ///   necessary.
  /// - `Err(...)` when the virtual path cannot be resolved to an existing
  ///   artifact.
  ///
  /// Resolution can fail when the path does not begin with a known source, or
  /// when a later path component does not correspond to an artifact produced by
  /// any decode node along the resolved branch.
  pub fn decodes_at_mut(
    &mut self,
    path: &VirtualPath,
  ) -> Result<&mut Vec<DecodeNode>, ManifestError> {
    let (source, remaining) = self
      .split_source(path)
      .ok_or_else(|| ManifestError::CouldNotResolve(path.to_string()))?;

    if remaining.is_empty() {
      return Ok(self.decodes.entry(source).or_default());
    }

    let Some(decodes) = self.decodes.get_mut(&source) else {
      return Err(ManifestError::CouldNotResolve(path.to_string()));
    };

    for decode in decodes.iter_mut() {
      if let Some(decodes) = decode.decodes_at_mut(&remaining) {
        return Ok(decodes);
      }
    }

    Err(ManifestError::CouldNotResolve(path.to_string()))
  }

  pub fn decodes_at(&self, path: &VirtualPath) -> Result<&[DecodeNode], ManifestError> {
    let (source, remaining) = self
      .split_source(path)
      .ok_or_else(|| ManifestError::CouldNotResolve(path.to_string()))?;

    if remaining.is_empty() {
      return Ok(self.decodes.get(&source).map(Vec::as_slice).unwrap_or(&[]));
    }

    let (node, output) = self.resolve_virtual_path(path)?;
    Ok(node.decodes.get(output).map(Vec::as_slice).unwrap_or(&[]))
  }

  /// Returns whether the artifact at the given virtual path is declared as an
  /// override.
  ///
  /// A path that resolves to nothing is simply not an override.
  pub fn is_overridden(&self, path: &VirtualPath) -> bool {
    self
      .resolve_virtual_path(path)
      .is_ok_and(|(node, output)| node.has_override_for(output))
  }

  #[allow(dead_code)]
  pub fn has_decodes(&self, path: &VirtualPath) -> bool {
    self
      .decodes_at(path)
      .is_ok_and(|decodes| !decodes.is_empty())
  }

  pub fn is_terminal(&self, path: &VirtualPath) -> bool {
    if self.sources.contains_key(Path::new(path.as_str())) {
      return !self.has_decodes(path);
    }
    self
      .resolve_virtual_path(path)
      .is_ok_and(|(node, output)| node.is_terminal_output(output))
  }

  /// Resolves a virtual path to the decode node that produced the artifact, and
  /// to the name of the output that node recorded it under.
  ///
  /// Only artifacts produced by a decode node resolve: a source, an implicit
  /// parent that is not itself an artifact, and any unknown path all fail rather
  /// than resolve to the artifact above them.
  ///
  /// # Errors
  ///
  /// Returns [`ManifestError::CouldNotResolve`] when the virtual path does not
  /// name a decoded output.
  pub fn resolve_virtual_path(
    &self,
    path: &VirtualPath,
  ) -> Result<(&DecodeNode, &str), ManifestError> {
    let (source, remaining) = self
      .split_source(path)
      .ok_or_else(|| ManifestError::CouldNotResolve(path.to_string()))?;

    if remaining.is_empty() {
      return Err(ManifestError::CouldNotResolve(path.to_string()));
    }

    for decode in self.decodes.get(&source).map(Vec::as_slice).unwrap_or(&[]) {
      if let Some(producer) = decode.decode_at(&remaining) {
        return Ok(producer);
      }
    }

    Err(ManifestError::CouldNotResolve(path.to_string()))
  }

  /// Resolves a virtual path to the decode node that produced the artifact, and
  /// to the name of the output that node recorded it under.
  ///
  /// This follows the same rules as [`Self::resolve_virtual_path`], and hands back
  /// the output name by value so that the decode node stays exclusively borrowed
  /// and can be amended by the caller.
  ///
  /// # Errors
  ///
  /// Returns [`ManifestError::CouldNotResolve`] when the virtual path does not
  /// name a decoded output.
  pub fn resolve_virtual_path_mut(
    &mut self,
    path: &VirtualPath,
  ) -> Result<(&mut DecodeNode, String), ManifestError> {
    let (source, remaining) = self
      .split_source(path)
      .ok_or_else(|| ManifestError::CouldNotResolve(path.to_string()))?;

    if remaining.is_empty() {
      return Err(ManifestError::CouldNotResolve(path.to_string()));
    }

    let Some(decodes) = self.decodes.get_mut(&source) else {
      return Err(ManifestError::CouldNotResolve(path.to_string()));
    };

    for decode in decodes.iter_mut() {
      if let Some(producer) = decode.resolve_output_mut(&remaining) {
        return Ok(producer);
      }
    }

    Err(ManifestError::CouldNotResolve(path.to_string()))
  }

  /// Splits a virtual path into the source the manifest records it under and the
  /// components left to resolve below that source.
  ///
  /// Sources are matched component by component, so `game.sfc` and `game.sfc.bak`
  /// are distinct and neither carries on past the other.
  ///
  /// Sources are held in a hash map, so two of them may name overlapping paths,
  /// as `roms/game.sfc` and `roms/game.sfc/extra` do. The longest matching name
  /// is the one a path is taken to belong to, which both makes the answer
  /// independent of the iteration order and keeps the most specific source from
  /// being shadowed by the one containing it.
  ///
  /// The source is returned by value rather than borrowed, so that a caller
  /// holding it may go on to borrow the manifest again.
  fn split_source<'p>(&self, path: &'p VirtualPath) -> Option<(String, Vec<&'p str>)> {
    let remaining: Vec<&str> = path.components().collect();

    let source = self
      .sources
      .keys()
      .filter_map(|source| {
        let source = source.to_str()?;
        let parts: Vec<&str> = source.split('/').collect();

        remaining
          .starts_with(&parts)
          .then_some((source, parts.len()))
      })
      .max_by_key(|(_, depth)| *depth)?;

    Some((source.0.to_owned(), remaining[source.1..].to_vec()))
  }

  /// Records a decode operation and the artifacts it produced.
  ///
  /// This is the logical half of adding a decode: the output names are checked,
  /// the decode is refused if it would collide with another one attached to the
  /// same input, and the [`DecodeNode`] is registered in memory. Nothing here
  /// touches the decode cache or the manifest file; storing the bytes and writing
  /// the manifest down belong to whoever owns them, which is the [`Project`] the
  /// operation was started from.
  ///
  /// # Errors
  ///
  /// Returns an [`AppError`] if the decode configuration is invalid
  /// or conflicts with the existing manifest.
  pub fn add_decode(
    &mut self,
    codecs: &CodecRegistry,
    path: &VirtualPath,
    codec_id: String,
    args: &Option<String>,
    name: String,
    artifacts: &[DecodedArtifact],
  ) -> Result<(), AppError> {
    if self.is_overridden(path) {
      return Err(OverrideError::OverriddenArtifactCannotBeDecoded(path.to_string()).into());
    }

    let kind = codecs
      .get(&codec_id)
      .map(Codec::kind)
      .ok_or_else(|| AppError::CodecUnavailable(codec_id.clone()))?;

    let decodes = self.decodes_at(path)?;

    let recorded: Vec<Option<CodecKind>> = decodes
      .iter()
      .map(|node| codecs.get(&node.codec.id).map(Codec::kind))
      .collect();

    if !recorded.is_empty()
      && (kind == CodecKind::Rebuild || recorded.contains(&Some(CodecKind::Rebuild)))
    {
      return Err(ManifestError::ConflictingDecodeKind(name, path.to_string()).into());
    }

    let existing: HashSet<&str> = decodes
      .iter()
      .flat_map(|decode| decode.outputs.iter())
      .map(String::as_str)
      .collect();

    let existing_names: HashSet<&str> = decodes.iter().map(|decode| decode.name.as_str()).collect();

    if existing_names.contains(name.as_str()) {
      return Err(ManifestError::DuplicateDecodeName(name, path.to_string()).into());
    }

    let mut new_outputs: HashSet<&str> = HashSet::new();
    for artifact in artifacts {
      if artifact.name.is_empty() {
        return Err(CodecError::ArtifactEmptyName.into());
      }
      if artifact.name.starts_with('/') {
        return Err(CodecError::ArtifactStartsWithSlash(artifact.name.clone()).into());
      }
      if artifact.name.ends_with('/') {
        return Err(CodecError::ArtifactEndsWithSlash(artifact.name.clone()).into());
      }
      if artifact.name.contains("//") {
        return Err(
          CodecError::ArtifactContainsInvalidSequence(artifact.name.clone(), "//".to_string())
            .into(),
        );
      }
      if let Some(component) = artifact
        .name
        .split('/')
        .find(|component| matches!(*component, "." | ".."))
      {
        return Err(
          CodecError::ArtifactContainsInvalidSequence(artifact.name.clone(), component.to_owned())
            .into(),
        );
      }
      if artifact.name.contains('\\') {
        return Err(
          CodecError::ArtifactContainsInvalidSequence(artifact.name.clone(), "\\".to_string())
            .into(),
        );
      }
      // The codec handing back one name twice is its own doing, whatever the
      // manifest already holds.
      if !new_outputs.insert(artifact.name.as_str()) {
        return Err(CodecError::DuplicateArtifact(artifact.name.clone()).into());
      }

      if existing.contains(artifact.name.as_str()) {
        return Err(ManifestError::DuplicateOutput(artifact.name.clone(), path.to_string()).into());
      }
    }

    let outputs: Vec<VirtualPath> = artifacts
      .iter()
      .map(|artifact| match VirtualPath::new(&artifact.name) {
        Ok(path) => path,
        Err(_) => unreachable!("Artifact names were validated above"),
      })
      .collect();

    // A path resolves to one artifact or to nothing, so the outputs sharing an
    // input artifact may not stand in an ancestor relation with one another. What
    // the pair is made of decides who is at fault: two outputs of this decode are
    // the codec's doing, while an output against one another decode already
    // declared is a conflict with the manifest.
    for (index, output) in outputs.iter().enumerate() {
      if let Some(other) = outputs[index + 1..].iter().find(|other| {
        // A name is its own ancestor, and being recorded twice is refused above.
        output != *other && (output.is_within(other) || other.is_within(output))
      }) {
        return Err(CodecError::OverlappingArtifacts(output.to_string(), other.to_string()).into());
      }
    }

    // Sorted, so that an artifact overlapping several of them is always reported
    // against the same one. A recorded output that no longer names a virtual path,
    // which only a hand-edited manifest produces, stands in no relation to
    // anything.
    let mut declared: Vec<&str> = existing.iter().copied().collect();
    declared.sort();
    let declared: Vec<VirtualPath> = declared
      .iter()
      .filter_map(|name| VirtualPath::new(name).ok())
      .collect();

    for output in &outputs {
      if let Some(other) = declared
        .iter()
        .find(|other| output != *other && (output.is_within(other) || other.is_within(output)))
      {
        return Err(
          ManifestError::OverlappingArtifacts(output.to_string(), other.to_string()).into(),
        );
      }
    }

    self.decodes_at_mut(path)?.push(DecodeNode {
      name,
      codec: CodecNode {
        id: codec_id,
        version: 1,
        // Recorded exactly as the codec was handed them. Nothing here parses,
        // validates or rewrites them: reading them back must reach `encode` the
        // same bytes `decode` was given.
        args: args.clone(),
      },
      outputs: artifacts.iter().map(|a| a.name.clone()).collect(),
      overrides: Vec::new(),
      decodes: HashMap::new(),
    });

    Ok(())
  }

  /// Removes a terminal decode from the manifest.
  ///
  /// `name` is required when multiple decodes share `path`.
  /// Returns the removed node so callers can clean up its cache and overrides.
  pub fn remove_decode(
    &mut self,
    path: &VirtualPath,
    name: Option<&str>,
  ) -> Result<DecodeNode, AppError> {
    let path_str = path.to_string();
    let decodes = self.decodes_at(path)?;

    let index = match name {
      Some(name) => decodes
        .iter()
        .position(|decode| decode.name == name)
        .ok_or_else(|| ManifestError::DecodeNotFound(name.to_owned(), path_str.clone()))?,
      None => match decodes.len() {
        0 => return Err(ManifestError::NoDecode(path_str).into()),
        1 => 0,
        _ => return Err(ManifestError::DecodeNameRequired(path_str).into()),
      },
    };

    if decodes[index].has_child_decodes() {
      return Err(ManifestError::DecodeNotTerminal(decodes[index].name.clone(), path_str).into());
    }

    let decodes = self.decodes_at_mut(path)?;
    let removed = decodes.remove(index);

    if decodes.is_empty() {
      let (source, remaining) = self
        .split_source(path)
        .ok_or_else(|| ManifestError::CouldNotResolve(path.to_string()))?;

      if remaining.is_empty() {
        self.decodes.remove(&source);
      } else {
        let (producer, output) = self.resolve_virtual_path_mut(path)?;
        producer.decodes.remove(&output);
      }
    }

    Ok(removed)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::codecs::ScriptEnvironment;
  use crate::project::overrides::errors::OverrideError;
  use crate::testing::fixtures::lorom;
  use crate::testing::fixtures::project::ProjectFixture;

  const FIRST_BANK: &str = "rom_bank_00.bin";

  /// A project whose manifest is written by hand, since the `overrides`
  /// declarations under test are exactly what the manifest records.
  ///
  /// The source name is randomised to keep each test's virtual paths to itself,
  /// and the cache is left empty: every case here reads the manifest only.
  fn fixture(manifest: &str) -> ProjectFixture {
    let (source, path) = ProjectFixture::random_source_name();

    let mut fixture = ProjectFixture::new();
    fixture.register_source_file(&path, b"HELLOWORLD");
    fixture.write_manifest(manifest.replace("$SOURCE", &source));
    fixture.reload();

    fixture
  }

  /// A manifest recording one source, whose decode node splits it in two banks
  /// and decodes the first of them into a single nested artifact.
  ///
  /// Each override declaration is passed separately, because an override belongs
  /// to the decode node that produced the artifact: `bank` is declared by the
  /// node producing the banks, `title` by the node producing the nested artifact.
  fn lorom_manifest(bank: &str, title: &str) -> String {
    format!(
      r#"{{
  "sources": {{ "$SOURCE": {{ "sha256": "unverified", "size": null, "label": null }} }},
  "decodes": {{
    "$SOURCE": [ {{
      "name": "rom_banks",
      "codec": {{ "id": "std/nintendo/snes/cart/lorom", "version": 1, "args": null }},
      "outputs": [ "{FIRST_BANK}", "rom_bank_01.bin" ],
      {bank}
      "decodes": {{
        "{FIRST_BANK}": [ {{
          "name": "TITLE",
          "codec": {{ "id": "std/generic/extract", "version": 1, "args": null }},
          "outputs": [ "TITLE.txt" ],
          {title}
          "decodes": {{}}
        }}]
      }}
    }}]
  }}
}}"#
    )
  }

  fn vpath(path: &str) -> VirtualPath {
    VirtualPath::new(path).unwrap()
  }

  /// The virtual paths a manifest method reported, as plain names.
  fn names(paths: Vec<VirtualPath>) -> Vec<String> {
    paths.iter().map(VirtualPath::to_string).collect()
  }

  #[test]
  fn lists_artifacts_the_same_way_however_sources_and_decodes_are_stored() {
    let manifest = r#"{
      "sources": {
        "b.sfc": { "sha256": "unverified", "size": null, "label": null },
        "a.sfc": { "sha256": "unverified", "size": null, "label": null }
      },
      "decodes": {
        "b.sfc": [ {
          "name": "second",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "tail.bin" ],
          "decodes": {}
        } ],
        "a.sfc": [ {
          "name": "first",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "head.bin" ],
          "decodes": {}
        } ]
      }
    }"#;

    // Sources and decode nodes are held in hash maps, so only the sort makes one
    // read agree with the next.
    for _ in 0..20 {
      let fixture = fixture(manifest);

      assert_eq!(
        names(fixture.project.manifest.artifacts()),
        [
          "a.sfc",
          "a.sfc/head.bin",
          "b.sfc",
          "b.sfc/tail.bin"
        ]
      );
    }
  }

  #[test]
  fn lists_the_outputs_of_the_decodes_nested_under_an_artifact() {
    // A decode hanging off an output produces artifacts below it, and an
    // override is declared by the node that produced what it names rather than by
    // any node above it.
    let manifest = r#"{
      "sources": { "$SOURCE": { "sha256": "unverified", "size": null, "label": null } },
      "decodes": { "$SOURCE": [ {
        "name": "rom_banks",
        "codec": { "id": "std/nintendo/snes/cart/lorom", "version": 1, "args": null },
        "outputs": [ "rom_bank_00.bin" ],
        "decodes": {
          "rom_bank_00.bin": [
            {
              "name": "gfx",
              "codec": { "id": "std/generic/extract", "version": 1, "args": null },
              "outputs": [ "data/gfx/head.bin", "data/sound.bin" ],
              "overrides": [ "data/gfx/head.bin" ],
              "decodes": {}
            },
            {
              "name": "title",
              "codec": { "id": "std/generic/extract", "version": 1, "args": null },
              "outputs": [ "TITLE.txt" ],
              "overrides": [ "TITLE.txt" ],
              "decodes": {}
            }
          ]
        }
      } ] }
    }"#;

    let fixture = fixture(manifest);
    let source = source_of(&fixture);

    assert_eq!(
      names(fixture.project.manifest.artifacts()),
      [
        source.clone(),
        format!("{source}/rom_bank_00.bin"),
        format!("{source}/rom_bank_00.bin/TITLE.txt"),
        format!("{source}/rom_bank_00.bin/data/gfx/head.bin"),
        format!("{source}/rom_bank_00.bin/data/sound.bin"),
      ]
    );

    // The bank neither nested decode hangs off is an artifact without being
    // overridden, and the two directories the nested names imply are not
    // artifacts at all.
    assert_eq!(
      names(fixture.project.manifest.overrides()),
      [
        format!("{source}/rom_bank_00.bin/TITLE.txt"),
        format!("{source}/rom_bank_00.bin/data/gfx/head.bin"),
      ]
    );
  }

  #[test]
  fn ignores_a_marker_a_decode_node_declares_for_an_artifact_it_did_not_produce() {
    // The declaration is local: naming the nested artifact from the node
    // producing its parent is not a way to override it.
    let fixture = fixture(&lorom_manifest(
      r#""overrides": [ "rom_bank_00.bin/TITLE.txt" ],"#,
      "",
    ));
    let source = source_of(&fixture);

    assert!(
      !fixture
        .project
        .manifest
        .is_overridden(&vpath(&lorom::title_vpath(&source)))
    );
  }

  #[test]
  fn resolves_nothing_for_a_source_or_for_an_implicit_parent() {
    let fixture = fixture(&lorom_manifest("", ""));
    let source = source_of(&fixture);
    let manifest = &fixture.project.manifest;

    for path in [
      // A source is never overridable.
      source.clone(),
      // A prefix of an artifact, but not one itself.
      format!("{source}/{FIRST_BANK}/TITLE"),
      // No such decode output at all.
      format!("{source}/missing.bin"),
      // Another source entirely.
      "elsewhere.sfc/rom_bank_01.bin".to_owned(),
    ] {
      assert!(
        manifest.resolve_virtual_path(&vpath(&path)).is_err(),
        "'{path}' should not resolve to a decoded artifact"
      );
      assert!(!manifest.is_overridden(&vpath(&path)));
    }
  }

  #[test]
  fn leaves_the_override_field_out_when_a_decode_node_declares_none() {
    // A decode node nobody overrode must be written back without the field, so
    // that saving a manifest does not rewrite the decode nodes it never touched.
    let node = DecodeNode {
      name: "head".to_owned(),
      codec: CodecNode::default(),
      outputs: vec!["head.bin".to_owned()],
      overrides: Vec::new(),
      decodes: HashMap::new(),
    };

    let json = serde_json::to_string(&node).unwrap();

    assert!(!json.contains("overrides"), "{json}");
  }

  #[test]
  fn persists_the_overrides_a_decode_node_declares() {
    let node = DecodeNode {
      name: "head".to_owned(),
      codec: CodecNode::default(),
      outputs: vec![
        "head.bin".to_owned(),
        "data/tail.bin".to_owned(),
      ],
      overrides: vec![
        "head.bin".to_owned(),
        "data/tail.bin".to_owned(),
      ],
      decodes: HashMap::new(),
    };

    let json = serde_json::to_string(&node).unwrap();
    let read: DecodeNode = serde_json::from_str(&json).unwrap();

    assert_eq!(
      read.overrides,
      [
        "head.bin",
        "data/tail.bin"
      ]
    );
    // The declaration round-trips as the output names it was recorded under.
    assert!(
      read
        .overrides
        .iter()
        .all(|name| read.outputs.contains(name))
    );
  }

  #[test]
  fn reads_a_decode_node_recording_no_override_field() {
    // `default` is what keeps a manifest written before overrides existed
    // readable now that the field is left out when empty.
    let json = r#"{
      "name": "head",
      "codec": { "id": "std/generic/extract", "version": 1, "args": null },
      "outputs": [ "head.bin" ],
      "decodes": {}
    }"#;

    let node: DecodeNode = serde_json::from_str(json).unwrap();

    assert!(node.overrides.is_empty());
  }

  #[test]
  fn lists_the_overrides_of_every_source_in_a_deterministic_order() {
    let manifest = r#"{
      "sources": {
        "b.sfc": { "sha256": "unverified", "size": null, "label": null },
        "a.sfc": { "sha256": "unverified", "size": null, "label": null }
      },
      "decodes": {
        "b.sfc": [ {
          "name": "second",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "tail.bin" ],
          "overrides": [ "tail.bin" ],
          "decodes": {}
        } ],
        "a.sfc": [ {
          "name": "first",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "head.bin" ],
          "overrides": [ "head.bin" ],
          "decodes": {}
        } ]
      }
    }"#;
    let fixture = fixture(manifest);

    // Sources are held in a hash map, so only the sort makes `override list` show
    // the same thing twice in a row.
    for _ in 0..20 {
      assert_eq!(
        names(fixture.project.manifest.overrides()),
        [
          "a.sfc/head.bin",
          "b.sfc/tail.bin"
        ]
      );
    }
  }

  #[test]
  fn resolves_an_artifact_under_a_source_registered_in_a_subdirectory() {
    // A source name may carry `/`-separated components of its own, which the
    // resolution has to consume whole before it looks at decode outputs.
    let manifest = r#"{
      "sources": {
        "roms/eu/game.sfc": { "sha256": "unverified", "size": null, "label": null },
        "roms/jp/game.sfc": { "sha256": "unverified", "size": null, "label": null }
      },
      "decodes": {
        "roms/eu/game.sfc": [ {
          "name": "extract",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "data/head.bin" ],
          "overrides": [ "data/head.bin" ],
          "decodes": {}
        } ],
        "roms/jp/game.sfc": [ {
          "name": "extract",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "data/head.bin" ],
          "overrides": [ "data/head.bin" ],
          "decodes": {}
        } ]
      }
    }"#;
    let fixture = fixture(manifest);

    for source in [
      "roms/eu/game.sfc",
      "roms/jp/game.sfc",
    ] {
      let path = vpath(&format!("{source}/data/head.bin"));
      let (node, output) = fixture
        .project
        .manifest
        .resolve_virtual_path(&path)
        .unwrap();

      // The marker is recorded under the nested output name, so it only reads
      // back as overridden if the whole name was resolved.
      assert_eq!(output, "data/head.bin");
      assert!(
        node.has_override_for(output),
        "'{path}' should be overridden"
      );
    }

    assert_eq!(
      names(fixture.project.manifest.overrides()),
      [
        "roms/eu/game.sfc/data/head.bin",
        "roms/jp/game.sfc/data/head.bin"
      ]
    );
  }

  #[test]
  fn resolves_nothing_for_a_path_that_shares_a_prefix_with_a_source() {
    let manifest = r#"{
      "sources": {
        "roms/eu/game.sfc": { "sha256": "unverified", "size": null, "label": null }
      },
      "decodes": {
        "roms/eu/game.sfc": [ {
          "name": "extract",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "head.bin" ],
          "decodes": {}
        } ]
      }
    }"#;
    let fixture = fixture(manifest);

    for path in [
      // The source on its own is not an artifact.
      "roms/eu/game.sfc",
      // A directory holding a source is not one either.
      "roms/eu",
      "roms",
      // A source sharing a prefix with the registered one, under another source.
      "roms/jp/game.sfc/head.bin",
      // A prefix of a decoded output.
      "roms/eu/game.sfc/head",
    ] {
      assert!(
        !fixture.project.manifest.is_overridden(&vpath(path)),
        "'{path}' should not resolve to an overridden artifact"
      );
    }
  }

  #[test]
  fn resolves_the_source_a_path_is_longest_matched_against() {
    // Both sources are a prefix of `roms/game.sfc/extra/head.bin`, so a
    // resolver taking whichever name it happened to meet first would only get
    // this wrong for some of the orders it is handed.
    let manifest = r#"{
      "sources": {
        "roms/game.sfc": { "sha256": "unverified", "size": null, "label": null },
        "roms/game.sfc/extra": { "sha256": "unverified", "size": null, "label": null }
      },
      "decodes": {
        "roms/game.sfc": [ {
          "name": "outer",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "extra/head.bin" ],
          "decodes": {}
        } ],
        "roms/game.sfc/extra": [ {
          "name": "inner",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "head.bin" ],
          "overrides": [ "head.bin" ],
          "decodes": {}
        } ]
      }
    }"#;

    // Sources live in a hash map, whose iteration order is randomised per map
    // rather than per process, so each round deserialises the manifest afresh and
    // is handed a fresh order. Over enough rounds, a resolver that does not
    // always take the longest match cannot pass.
    for _ in 0..100 {
      let manifest: Manifest = serde_json::from_str(manifest).unwrap();
      let path = vpath("roms/game.sfc/extra/head.bin");

      // The longest name owns the path, so this is an output of `inner` rather
      // than the nested `extra/head.bin` of `outer`.
      assert!(
        manifest.is_overridden(&path),
        "'{path}' should resolve to the deepest source"
      );
      assert_eq!(
        names(manifest.overrides()),
        ["roms/game.sfc/extra/head.bin"]
      );
    }
  }

  #[test]
  fn resolves_the_shorter_source_for_a_path_it_alone_owns() {
    let manifest = r#"{
      "sources": {
        "roms/game.sfc": { "sha256": "unverified", "size": null, "label": null },
        "roms/game.sfc/extra": { "sha256": "unverified", "size": null, "label": null }
      },
      "decodes": {
        "roms/game.sfc": [ {
          "name": "outer",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "head.bin", "extra/head.bin" ],
          "overrides": [ "head.bin" ],
          "decodes": {}
        } ],
        "roms/game.sfc/extra": [ {
          "name": "inner",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "head.bin" ],
          "decodes": {}
        } ]
      }
    }"#;
    let fixture = fixture(manifest);

    assert!(
      fixture
        .project
        .manifest
        .is_overridden(&vpath("roms/game.sfc/head.bin"))
    );
    assert_eq!(
      names(fixture.project.manifest.overrides()),
      ["roms/game.sfc/head.bin"]
    );
  }

  #[test]
  fn tells_a_source_name_apart_from_a_longer_name_sharing_its_text() {
    // `game.sfc.bak` merely starts like `game.sfc`, and is a different source.
    let manifest = r#"{
      "sources": {
        "game.sfc": { "sha256": "unverified", "size": null, "label": null },
        "game.sfc.bak": { "sha256": "unverified", "size": null, "label": null }
      },
      "decodes": {
        "game.sfc": [ {
          "name": "current",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "head.bin" ],
          "overrides": [ "head.bin" ],
          "decodes": {}
        } ],
        "game.sfc.bak": [ {
          "name": "backup",
          "codec": { "id": "std/generic/extract", "version": 1, "args": null },
          "outputs": [ "head.bin" ],
          "decodes": {}
        } ]
      }
    }"#;
    let fixture = fixture(manifest);

    // Each source resolves the artifacts recorded against it, and only those.
    assert!(
      fixture
        .project
        .manifest
        .is_overridden(&vpath("game.sfc/head.bin"))
    );
    assert!(
      !fixture
        .project
        .manifest
        .is_overridden(&vpath("game.sfc.bak/head.bin"))
    );
    assert_eq!(
      names(fixture.project.manifest.overrides()),
      ["game.sfc/head.bin"]
    );
  }

  #[test]
  fn accepts_a_decode_node_naming_its_outputs_the_way_a_virtual_path_reads() {
    // Dots are ordinary characters, including a component that is nothing but
    // one, and the directories an output name implies are not artifacts.
    let outputs = [
      "file..bin",
      ".hidden",
      "file.bin",
      "data/gfx/file.bin",
      "data/gfx/deep/nested/file.bin",
    ];

    // Nothing on disk is involved: the manifest records the decode on its own.
    let (source, mut recorded) = source_manifest("[]");
    let path = VirtualPath::new(&source).unwrap();
    let artifacts: Vec<DecodedArtifact> = outputs
      .iter()
      .map(|name| DecodedArtifact {
        name: (*name).to_owned(),
        data: b"HELLO".to_vec(),
      })
      .collect();

    let scripts = tempfile::tempdir().unwrap();
    let codecs =
      CodecRegistry::new(scripts.path().to_path_buf(), ScriptEnvironment::default()).unwrap();
    recorded
      .add_decode(
        &codecs,
        &path,
        "std/generic/extract".to_owned(),
        &None,
        "head".to_owned(),
        &artifacts,
      )
      .unwrap();

    // Every accepted name is an artifact in its own right, and each resolves back
    // to the decode node that was just recorded.
    let mut expected = vec![source.clone()];
    expected.extend(outputs.iter().map(|name| format!("{source}/{name}")));
    expected.sort();

    assert_eq!(names(recorded.artifacts()), expected);

    for name in outputs {
      let path = VirtualPath::new(&format!("{source}/{name}")).unwrap();
      let (node, output) = recorded.resolve_virtual_path(&path).unwrap();

      assert_eq!(node.name, "head");
      assert_eq!(output, name);
    }
  }

  #[test]
  fn refuses_to_decode_into_an_overridden_artifact() {
    let mut fixture = fixture(&lorom_manifest(
      r#""overrides": [ "rom_bank_01.bin" ],"#,
      "",
    ));
    let source = source_of(&fixture);
    let path = vpath(&format!("{source}/rom_bank_01.bin"));
    let artifacts = vec![DecodedArtifact {
      name: "inner.bin".to_owned(),
      data: b"HELLO".to_vec(),
    }];

    let scripts = tempfile::tempdir().unwrap();
    let codecs =
      CodecRegistry::new(scripts.path().to_path_buf(), ScriptEnvironment::default()).unwrap();
    let err = fixture
      .project
      .manifest
      .add_decode(
        &codecs,
        &path,
        "std/generic/extract".to_owned(),
        &None,
        "inner".to_owned(),
        &artifacts,
      )
      .unwrap_err();

    match err {
      AppError::OverrideError(OverrideError::OverriddenArtifactCannotBeDecoded(reported)) => {
        assert_eq!(reported, path.to_string());
      }
      err => panic!("Unexpected error: {err:?}"),
    }

    let message = OverrideError::OverriddenArtifactCannotBeDecoded(path.to_string()).to_string();

    assert!(message.contains(&path.to_string()), "{message}");
  }

  /// A manifest whose source already carries one decode producing `existing`.
  ///
  /// Returned alongside the source name, since the path every decode under test
  /// hangs off is the source itself.
  fn manifest_with_decode(existing: &[&str]) -> (String, Manifest) {
    let (source, _) = ProjectFixture::random_source_name();

    let outputs = existing
      .iter()
      .map(|output| format!("\"{output}\""))
      .collect::<Vec<String>>()
      .join(", ");

    let manifest: Manifest = serde_json::from_str(&format!(
      r#"{{
  "sources": {{ "{source}": {{ "sha256": "unverified", "size": null, "label": null }} }},
  "decodes": {{ "{source}": [
    {{
      "name": "sibling",
      "codec": {{ "id": "std/generic/extract", "version": 1, "args": null }},
      "outputs": [ {outputs} ],
      "decodes": {{}}
    }}
  ] }}
}}"#
    ))
    .unwrap();

    (source, manifest)
  }

  /// The artifacts a decode handing back `outputs` would produce.
  fn artifacts_of(outputs: &[&str]) -> Vec<DecodedArtifact> {
    outputs
      .iter()
      .map(|name| DecodedArtifact {
        name: (*name).to_owned(),
        data: b"HELLO".to_vec(),
      })
      .collect()
  }

  /// Declares a decode producing `outputs` on the source of `manifest`.
  fn declaring(source: &str, manifest: &mut Manifest, outputs: &[&str]) -> Result<(), AppError> {
    declaring_codec(source, manifest, "std/generic/extract", outputs)
  }

  /// Declares a decode by `codec` producing `outputs` on the source of
  /// `manifest`.
  fn declaring_codec(
    source: &str,
    manifest: &mut Manifest,
    codec: &str,
    outputs: &[&str],
  ) -> Result<(), AppError> {
    declaring_named(source, manifest, codec, "conflicting", outputs)
  }

  /// Declares a decode by `codec`, under the name `name`, producing `outputs` on
  /// the source of `manifest`.
  fn declaring_named(
    source: &str,
    manifest: &mut Manifest,
    codec: &str,
    name: &str,
    outputs: &[&str],
  ) -> Result<(), AppError> {
    let scripts = tempfile::tempdir().unwrap();
    let codecs =
      CodecRegistry::new(scripts.path().to_path_buf(), ScriptEnvironment::default()).unwrap();
    manifest.add_decode(
      &codecs,
      &VirtualPath::new(source).unwrap(),
      codec.to_owned(),
      &None,
      name.to_owned(),
      &artifacts_of(outputs),
    )
  }

  /// The banks the LoROM rebuild codec produces, and the window the extraction
  /// patch codec produces, so that each may stand in for the other.
  const BANKS: [&str; 1] = ["rom_bank_00.bin"];
  const WINDOW: [&str; 1] = ["window.bin"];

  #[test]
  fn refuses_a_decode_whose_kind_conflicts_with_the_ones_already_on_the_artifact() {
    let lorom = "std/nintendo/snes/cart/lorom";
    let extract = "std/generic/extract";

    // A rebuild produces the artifact whole and so has it to itself, while a
    // patch edits what is already there and so is one of several.
    for (first, first_outputs, second, second_outputs) in [
      // A patch cannot follow a rebuild...
      (lorom, &BANKS[..], extract, &WINDOW[..]),
      // ...nor a rebuild follow a patch...
      (extract, &WINDOW[..], lorom, &BANKS[..]),
      // ...nor a rebuild follow another rebuild.
      (lorom, &BANKS[..], lorom, &BANKS[..]),
    ] {
      let (source, mut manifest) = source_manifest("[]");

      declaring_codec(&source, &mut manifest, first, first_outputs).unwrap();
      let before = names(manifest.artifacts());

      match declaring_named(&source, &mut manifest, second, "second", second_outputs).unwrap_err() {
        AppError::ManifestError(ManifestError::ConflictingDecodeKind(name, under)) => {
          assert_eq!((name.as_str(), under.as_str()), ("second", source.as_str()));
        }
        err => panic!("Unexpected error for '{second}' after '{first}': {err:?}"),
      }

      // The refused decode left the manifest with only the first one on it.
      assert_eq!(names(manifest.artifacts()), before);
    }
  }

  #[test]
  fn accepts_several_patches_of_the_same_artifact() {
    let extract = "std/generic/extract";
    let (source, mut manifest) = source_manifest("[]");

    // Patching an artifact is the one case where several decodes share it: each
    // is applied over the last, so the order they are declared in decides what
    // the artifact ends up holding.
    for window in [
      "head.bin",
      "middle.bin",
      "tail.bin",
    ] {
      declaring_named(&source, &mut manifest, extract, window, &[window]).unwrap();
    }

    assert_eq!(
      names(manifest.artifacts()),
      [
        source.clone(),
        format!("{source}/head.bin"),
        format!("{source}/middle.bin"),
        format!("{source}/tail.bin"),
      ]
    );
  }

  #[test]
  fn refuses_a_decode_naming_a_codec_the_registry_does_not_hold() {
    let (source, mut manifest) = source_manifest("[]");

    // The kind comes off the codec, so a decode naming none cannot be recorded
    // with one.
    match declaring_codec(&source, &mut manifest, "std/generic/absent", &BANKS).unwrap_err() {
      AppError::CodecUnavailable(id) => assert_eq!(id, "std/generic/absent"),
      err => panic!("Unexpected error: {err:?}"),
    }

    assert_eq!(names(manifest.artifacts()), [source]);
  }

  #[test]
  fn refuses_a_decode_node_naming_an_output_no_virtual_path_could_hold() {
    // Each name is refused by its own case, with the offending name carried
    // through: a name no virtual path can represent is never recorded.
    for name in [
      "",
      "/file.bin",
      "file.bin/",
      "data//file.bin",
      "data/./file.bin",
      "data/../file.bin",
      r"data\file.bin",
    ] {
      let (source, mut manifest) = source_manifest("[]");
      let before = names(manifest.artifacts());

      // Each name must be refused for the reason it is invalid, and no other,
      // carrying that name back out.
      match declaring(&source, &mut manifest, &[name]).unwrap_err() {
        AppError::CodecError(CodecError::ArtifactEmptyName) => assert_eq!(name, ""),
        AppError::CodecError(CodecError::ArtifactStartsWithSlash(declared)) => {
          assert_eq!((name, declared.as_str()), ("/file.bin", "/file.bin"));
        }
        AppError::CodecError(CodecError::ArtifactEndsWithSlash(declared)) => {
          assert_eq!((name, declared.as_str()), ("file.bin/", "file.bin/"));
        }
        AppError::CodecError(CodecError::ArtifactContainsInvalidSequence(declared, sequence)) => {
          assert_eq!(
            (name, declared.as_str(), sequence.as_str()),
            match name {
              "data//file.bin" => ("data//file.bin", "data//file.bin", "//"),
              "data/./file.bin" => ("data/./file.bin", "data/./file.bin", "."),
              "data/../file.bin" => ("data/../file.bin", "data/../file.bin", ".."),
              r"data\file.bin" => (r"data\file.bin", r"data\file.bin", r"\"),
              name => panic!("Unexpected name: {name}"),
            }
          );
        }
        err => panic!("Unexpected error for '{name}': {err:?}"),
      }

      assert_eq!(names(manifest.artifacts()), before);
    }
  }

  #[test]
  fn blames_the_codec_for_a_decode_repeating_one_of_its_own_outputs() {
    let (source, mut manifest) = manifest_with_decode(&["elsewhere.bin"]);
    let before = names(manifest.artifacts());

    match declaring(&source, &mut manifest, &["foo.bin", "foo.bin"]).unwrap_err() {
      AppError::CodecError(CodecError::DuplicateArtifact(name)) => {
        assert_eq!(name, "foo.bin");
      }
      err => panic!("Unexpected error: {err:?}"),
    }

    assert_eq!(names(manifest.artifacts()), before);
  }

  #[test]
  fn blames_the_codec_for_its_own_outputs_overlapping_each_other() {
    for (first, second) in [
      ("data", "data/foo.bin"),
      ("data/foo.bin", "data"),
      ("data/gfx", "data/gfx/head.bin"),
      ("data/gfx/head.bin", "data/gfx"),
    ] {
      let (source, mut manifest) = manifest_with_decode(&["elsewhere.bin"]);
      let before = names(manifest.artifacts());

      match declaring(&source, &mut manifest, &[first, second]).unwrap_err() {
        AppError::CodecError(CodecError::OverlappingArtifacts(left, right)) => {
          assert_eq!((left.as_str(), right.as_str()), (first, second));
        }
        err => panic!("Unexpected error for '{first}' and '{second}': {err:?}"),
      }

      assert_eq!(names(manifest.artifacts()), before);
    }
  }

  #[test]
  fn blames_the_manifest_for_an_output_already_declared() {
    let (source, mut manifest) = manifest_with_decode(&["foo.bin"]);
    let before = names(manifest.artifacts());

    match declaring(&source, &mut manifest, &["foo.bin"]).unwrap_err() {
      AppError::ManifestError(ManifestError::DuplicateOutput(name, under)) => {
        assert_eq!(name, "foo.bin");
        assert_eq!(under, source);
      }
      err => panic!("Unexpected error: {err:?}"),
    }

    assert_eq!(names(manifest.artifacts()), before);
  }

  #[test]
  fn blames_the_manifest_for_an_output_overlapping_what_is_declared() {
    for (existing, new) in [
      // Either one may be the ancestor of the other.
      ("data", "data/foo.bin"),
      ("data/foo.bin", "data"),
      ("data/gfx", "data/gfx/head.bin"),
      ("data/gfx/head.bin", "data/gfx"),
    ] {
      let (source, mut manifest) = manifest_with_decode(&[existing]);
      let before = names(manifest.artifacts());

      match declaring(&source, &mut manifest, &[new]).unwrap_err() {
        AppError::ManifestError(ManifestError::OverlappingArtifacts(left, right)) => {
          assert_eq!((left.as_str(), right.as_str()), (new, existing));
        }
        err => panic!("Unexpected error for '{new}' over '{existing}': {err:?}"),
      }

      assert_eq!(names(manifest.artifacts()), before);
    }
  }

  #[test]
  fn accepts_outputs_that_only_share_directories_or_text() {
    let accepted: Vec<Vec<&str>> = vec![
      // Distinct artifacts under the same implicit directory.
      vec![
        "data/foo.bin",
        "data/bar.bin",
      ],
      vec![
        "data/gfx/head.bin",
        "data/gfx/tail.bin",
        "data/sound/music.bin",
      ],
      // Neighbouring names that happen to share their beginning.
      vec![
        "head.bin",
        "head.bin.bak",
      ],
      // A dot inside a component is not a directory of its own.
      vec![
        "file..bin",
        ".hidden",
      ],
      // A whole subtree beside a sibling that is not its parent.
      vec![
        "data/gfx/head.bin",
        "data/sound.bin",
      ],
    ];

    for outputs in accepted {
      let (source, mut manifest) = manifest_with_decode(&["elsewhere.bin"]);

      declaring(&source, &mut manifest, &outputs)
        .unwrap_or_else(|err| panic!("{outputs:?} should be accepted: {err:?}"));

      let mut expected = names(manifest.artifacts());
      expected.sort();

      assert_eq!(
        manifest
          .artifacts()
          .iter()
          .map(VirtualPath::to_string)
          .collect::<Vec<String>>(),
        expected
      );
    }
  }

  #[test]
  fn removes_the_only_decode_of_an_artifact_without_a_name() {
    let (source, mut manifest) = source_manifest(
      r#"[ {
        "name": "head",
        "codec": { "id": "std/generic/extract", "version": 1, "args": null },
        "outputs": [ "head.bin" ],
        "decodes": {}
      } ]"#,
    );

    let removed = manifest.remove_decode(&vpath(&source), None).unwrap();

    assert_eq!(removed.name, "head");
    assert_eq!(removed.outputs, ["head.bin"]);
    assert!(removed.overrides.is_empty());

    // The output is gone with its recipe, and the empty source bucket too.
    assert_eq!(names(manifest.artifacts()), vec![source.to_owned()]);
    assert!(!manifest.decodes.contains_key(&source));
  }

  #[test]
  fn requires_a_name_when_several_decodes_share_an_artifact() {
    let (source, mut manifest) = source_manifest(
      r#"[ {
        "name": "head",
        "codec": { "id": "std/generic/extract", "version": 1, "args": null },
        "outputs": [ "head.bin" ],
        "decodes": {}
      }, {
        "name": "tail",
        "codec": { "id": "std/generic/extract", "version": 1, "args": null },
        "outputs": [ "tail.bin" ],
        "decodes": {}
      } ]"#,
    );

    match manifest.remove_decode(&vpath(&source), None).unwrap_err() {
      AppError::ManifestError(ManifestError::DecodeNameRequired(reported)) => {
        assert_eq!(reported, source);
      }
      err => panic!("Unexpected error: {err:?}"),
    }

    // Nothing was selected, so nothing was removed.
    assert_eq!(
      names(manifest.artifacts()),
      [
        source.clone(),
        format!("{source}/head.bin"),
        format!("{source}/tail.bin"),
      ]
    );

    // A name selects the decode to remove and leaves its sibling alone.
    let removed = manifest
      .remove_decode(&vpath(&source), Some("tail"))
      .unwrap();

    assert_eq!(removed.name, "tail");
    assert_eq!(removed.outputs, ["tail.bin"]);
    assert_eq!(
      names(manifest.artifacts()),
      [
        source.clone(),
        format!("{source}/head.bin"),
      ]
    );
  }

  #[test]
  fn refuses_to_remove_a_decode_with_child_decodes() {
    let (source, mut manifest) = source_manifest(
      r#"[ {
        "name": "head",
        "codec": { "id": "std/generic/extract", "version": 1, "args": null },
        "outputs": [ "head.bin" ],
        "decodes": {
          "head.bin": [ {
            "name": "TITLE",
            "codec": { "id": "std/generic/extract", "version": 1, "args": null },
            "outputs": [ "TITLE.txt" ],
            "decodes": {}
          } ]
        }
      } ]"#,
    );

    match manifest.remove_decode(&vpath(&source), None).unwrap_err() {
      AppError::ManifestError(ManifestError::DecodeNotTerminal(name, under)) => {
        assert_eq!((name.as_str(), under.as_str()), ("head", source.as_str()));
      }
      err => panic!("Unexpected error: {err:?}"),
    }

    assert_eq!(
      names(manifest.artifacts()),
      [
        source.clone(),
        format!("{source}/head.bin"),
        format!("{source}/head.bin/TITLE.txt"),
      ]
    );

    // The terminal decode below it can go, taking only its own bucket with it.
    let removed = manifest
      .remove_decode(&vpath(&format!("{source}/head.bin")), None)
      .unwrap();

    assert_eq!(removed.name, "TITLE");
    assert_eq!(removed.outputs, ["TITLE.txt"]);
    assert_eq!(
      names(manifest.artifacts()),
      [
        source.clone(),
        format!("{source}/head.bin"),
      ]
    );

    let (head, _) = manifest
      .resolve_virtual_path(&vpath(&format!("{source}/head.bin")))
      .unwrap();
    assert!(!head.decodes.contains_key("head.bin"));
  }

  #[test]
  fn reports_a_name_no_decode_on_the_artifact_has() {
    let (source, mut manifest) = source_manifest(
      r#"[ {
        "name": "head",
        "codec": { "id": "std/generic/extract", "version": 1, "args": null },
        "outputs": [ "head.bin" ],
        "decodes": {}
      } ]"#,
    );

    match manifest
      .remove_decode(&vpath(&source), Some("missing"))
      .unwrap_err()
    {
      AppError::ManifestError(ManifestError::DecodeNotFound(name, under)) => {
        assert_eq!(
          (name.as_str(), under.as_str()),
          ("missing", source.as_str())
        );
      }
      err => panic!("Unexpected error: {err:?}"),
    }

    assert_eq!(
      names(manifest.artifacts()),
      [
        source.clone(),
        format!("{source}/head.bin"),
      ]
    );
  }

  /// A manifest holding one registered source, with `decodes` recorded against it.
  ///
  /// Returned alongside the source name, since every virtual path under test
  /// hangs off it.
  fn source_manifest(decodes: &str) -> (String, Manifest) {
    let (source, _) = ProjectFixture::random_source_name();

    let manifest: Manifest = serde_json::from_str(&format!(
      r#"{{
  "sources": {{ "{source}": {{ "sha256": "unverified", "size": null, "label": null }} }},
  "decodes": {{ "{source}": {decodes} }}
}}"#
    ))
    .unwrap();

    (source, manifest)
  }

  /// The source a fixture registered, read back from its manifest.
  fn source_of(fixture: &ProjectFixture) -> String {
    fixture
      .project
      .manifest
      .sources
      .keys()
      .next()
      .unwrap()
      .to_str()
      .unwrap()
      .to_owned()
  }
}
