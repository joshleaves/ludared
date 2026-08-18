use crate::codecs::DecodedArtifact;
use crate::errors::app_error::AppError::{self, ManifestFileJson};
use crate::manifest::decode_node::CodecNode;
use crate::project::Project;
use crate::source::Source;
use crate::virtual_path::VirtualPath;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(crate) mod decode_node;
pub(crate) mod errors;
use decode_node::DecodeNode;
use errors::ManifestError;

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct Manifest {
  #[serde(default)]
  pub sources: HashMap<PathBuf, Source>,
  #[serde(default)]
  pub decodes: HashMap<String, Vec<DecodeNode>>,
}

impl Manifest {
  pub(crate) fn load(path: &Path) -> Result<Self, AppError> {
    let content = std::fs::read_to_string(path).map_err(AppError::ManifestFileIo)?;
    // you may want a Json variant later
    let manifest: Self = serde_json::from_str(&content).map_err(AppError::ManifestFileJson)?;
    Ok(manifest)
  }

  pub(crate) fn save(&self, path: &Path) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(self).map_err(AppError::ManifestFileJson)?;
    std::fs::write(path, json).map_err(AppError::ManifestFileIo)?;
    Ok(())
  }

  /// Returns the decode nodes attached to the artifact at the given virtual path.
  ///
  /// An existing artifact with no attached decode nodes returns an empty slice.
  /// An error is returned if the virtual path cannot be resolved to an existing
  /// source or decoded artifact.
  ///
  /// Artifact paths may contain multiple path components. These are resolved as
  /// a single artifact when they correspond to an output produced by a decode
  /// node.
  ///
  /// # Returns
  ///
  /// - `Ok(decodes)` when the artifact exists and has one or more decode nodes.
  /// - `Ok(&[])` when the artifact exists but has no decode nodes attached.
  /// - `Err(...)` when the virtual path cannot be resolved to an existing artifact.
  ///
  /// Resolution can fail when the path does not begin with a known source, or
  /// when a later path component does not correspond to an artifact produced by
  /// any decode node along the resolved branch.
  pub(crate) fn get_decodes(&self, path: &VirtualPath) -> Result<&[DecodeNode], ManifestError> {
    let remaining: Vec<&str> = path.components().collect();

    let Some((source, source_depth)) = self.sources.keys().find_map(|source| {
      let source = source.to_str()?;
      let components: Vec<&str> = source.split('/').collect();

      remaining
        .starts_with(&components)
        .then(|| (source.to_owned(), components.len()))
    }) else {
      return Err(ManifestError::CouldNotResolve(path.to_string()));
    };

    let decodes = self.decodes.get(&source).map(Vec::as_slice).unwrap_or(&[]);

    let remaining = &remaining[source_depth..];

    if remaining.is_empty() {
      return Ok(decodes);
    }

    for decode in decodes {
      if let Some(decodes) = decode.get_decodes(remaining) {
        return Ok(decodes);
      }
    }

    Err(ManifestError::CouldNotResolve(path.to_string()))
  }

  /// Returns the mutable decode nodes attached to the artifact at the given
  /// virtual path.
  ///
  /// This method follows the same resolution rules as [`Self::get_decodes`], but
  /// returns mutable access to the decode nodes. It never creates decode buckets.
  ///
  /// # Returns
  ///
  /// - `Ok(decodes)` when the artifact exists and has one or more decode nodes.
  /// - `Ok(&mut [])` when the artifact exists and has an existing but empty
  ///   decode bucket.
  /// - `Err(...)` when the virtual path cannot be resolved to an existing
  ///   artifact, or when the artifact has no decode bucket.
  ///
  /// Resolution can fail when the path does not begin with a known source, or
  /// when a later path component does not correspond to an artifact produced by
  /// any decode node along the resolved branch.
  pub(crate) fn get_decodes_mut(
    &mut self,
    path: &VirtualPath,
  ) -> Result<&mut [DecodeNode], ManifestError> {
    let remaining: Vec<&str> = path.components().collect();

    let Some((source, source_depth)) = self.sources.keys().find_map(|source| {
      let source = source.to_str()?;
      let components: Vec<&str> = source.split('/').collect();

      remaining
        .starts_with(&components)
        .then(|| (source.to_owned(), components.len()))
    }) else {
      return Err(ManifestError::CouldNotResolve(path.to_string()));
    };

    let remaining = &remaining[source_depth..];

    let Some(decodes) = self.decodes.get_mut(&source) else {
      return Err(ManifestError::CouldNotResolve(path.to_string()));
    };

    if remaining.is_empty() {
      return Ok(decodes);
    }

    for decode in decodes {
      if let Some(decodes) = decode.get_decodes_mut(remaining) {
        return Ok(decodes);
      }
    }

    Err(ManifestError::CouldNotResolve(path.to_string()))
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
  pub(crate) fn get_or_create_decodes_mut(
    &mut self,
    path: &VirtualPath,
  ) -> Result<&mut Vec<DecodeNode>, ManifestError> {
    let remaining: Vec<&str> = path.components().collect();

    let Some((source, source_depth)) = self.sources.keys().find_map(|source| {
      let source = source.to_str()?;
      let components: Vec<&str> = source.split('/').collect();

      remaining
        .starts_with(&components)
        .then(|| (source.to_owned(), components.len()))
    }) else {
      return Err(ManifestError::CouldNotResolve(path.to_string()));
    };

    let remaining = &remaining[source_depth..];

    if remaining.is_empty() {
      return Ok(self.decodes.entry(source).or_default());
    }

    let Some(decodes) = self.decodes.get_mut(&source) else {
      return Err(ManifestError::CouldNotResolve(path.to_string()));
    };

    for decode in decodes {
      if let Some(decodes) = decode.get_or_create_decodes_mut(remaining) {
        return Ok(decodes);
      }
    }

    Err(ManifestError::CouldNotResolve(path.to_string()))
  }

  pub(crate) fn add_decode(
    &mut self,
    path: &VirtualPath,
    codec_id: String,
    args: &Option<String>,
    name: String,
    artifacts: Vec<DecodedArtifact>,
  ) -> Result<(), AppError> {
    let decodes = self.get_or_create_decodes_mut(path)?;

    let existing: HashSet<&str> = decodes
      .iter()
      .flat_map(|decode| decode.outputs.iter())
      .map(String::as_str)
      .collect();

    let existing_names: HashSet<&str> = decodes.iter().map(|decode| decode.name.as_str()).collect();

    if existing_names.contains(name.as_str()) {
      return Err(ManifestError::DuplicateDecodeName(name, path.to_string()).into());
    }

    // TODO: Defensive programming is nice, but it's still double-responsibility. Rethink that!
    let mut new_outputs: HashSet<&str> = HashSet::new();
    for artifact in &artifacts {
      if !new_outputs.insert(artifact.name.as_str()) {
        return Err(ManifestError::DuplicateArtifact(artifact.name.clone()).into());
      }

      if existing.contains(artifact.name.as_str()) {
        return Err(ManifestError::DuplicateOutput(artifact.name.clone(), path.to_string()).into());
      }
    }

    let mut project = Project::load_default()?;
    for artifact in &artifacts {
      project
        .cache
        .add_entry(path.join(&artifact.name)?, &artifact.data)?;
    }
    let args_json = match args {
      Some(args) => match serde_json::from_str(args) {
        Ok(v) => v,
        Err(e) => return Err(ManifestFileJson(e)),
      },
      None => serde_json::Value::Object(Default::default()),
    };

    decodes.push(DecodeNode {
      name,
      codec: CodecNode {
        id: codec_id,
        version: 1,
        args: args_json,
      },
      outputs: artifacts.iter().map(|a| a.name.clone()).collect(),
      decodes: HashMap::new(),
    });
    project.cache.save()?;

    Ok(())
  }
}
