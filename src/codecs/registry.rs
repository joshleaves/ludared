use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::cdrom::iso9660::Iso9660;
use crate::codecs::cdrom::mode2352::Mode2352;
use crate::codecs::errors::CodecError;
use crate::codecs::generic::extract::Extract;
use crate::codecs::generic::split::Split;
use crate::codecs::nintendo::n3ds::container::cci::N3dsCci;
use crate::codecs::nintendo::snes::cart_exhirom::SnesExHiRom;
use crate::codecs::nintendo::snes::cart_hirom::SnesHiRom;
use crate::codecs::nintendo::snes::cart_lorom::SnesLoRom;
use crate::codecs::nintendo::snes::cart_sa1::SnesSa1Rom;
use crate::codecs::nintendo::snes::cart_sdd1::SnesSdd1Rom;
use crate::codecs::nintendo::snes::cart_spc7110::SnesSpc7110Rom;
use crate::codecs::scripts::lua::LuaCodec;
use crate::scripts::errors::ScriptError;
use log::*;
use std::path::PathBuf;

const SHARED_DIRECTORIES: &[&str] = &[
  "shared",
  "libraries",
];

static BUILTINS: &[&dyn Codec] = &[
  &N3dsCci,
  &Extract,
  &Iso9660,
  &Mode2352,
  &Split,
  &SnesExHiRom,
  &SnesLoRom,
  &SnesHiRom,
  &SnesSa1Rom,
  &SnesSdd1Rom,
  &SnesSpc7110Rom,
];

pub(crate) struct CodecRegistry {
  scripts_root: PathBuf,
  scripts: Vec<Box<dyn Codec>>,
}

impl CodecRegistry {
  pub fn new(scripts_root: PathBuf) -> Result<Self, ScriptError> {
    let mut registry = Self {
      scripts_root,
      scripts: Vec::new(),
    };

    registry.discover_scripts()?;

    Ok(registry)
  }

  fn discover_scripts(&mut self) -> Result<(), ScriptError> {
    if !self.scripts_root.exists() {
      return Ok(());
    }

    let lua = mlua::Lua::new();
    let mut pending = vec![PathBuf::new()];

    while let Some(prefix) = pending.pop() {
      let directory = self.scripts_root.join(&prefix);
      let entries = std::fs::read_dir(&directory).map_err(|err| {
        std::io::Error::new(
          err.kind(),
          format!(
            "Cannot open scripts folder '{}': {err}",
            directory.display()
          ),
        )
      })?;

      for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let relative = prefix.join(entry.file_name());

        if file_type.is_dir() {
          if !SHARED_DIRECTORIES.contains(&entry.file_name().to_string_lossy().as_ref()) {
            pending.push(relative);
          }
          continue;
        }

        if !file_type.is_file() {
          continue;
        }

        match relative.extension().and_then(|ext| ext.to_str()) {
          Some("lua") => {
            let path = self.scripts_root.join(&relative);
            let id = format!("scripts/{}", relative.to_string_lossy());

            let codec = LuaCodec::new(&lua, path, id)?;

            self.scripts.push(Box::new(codec));
          }
          _ => continue,
        }
      }
    }

    Ok(())
  }

  pub fn all_codecs(&self) -> impl Iterator<Item = &dyn Codec> {
    BUILTINS
      .iter()
      .copied()
      .chain(self.scripts.iter().map(|codec| codec.as_ref()))
  }

  pub fn get(&self, id: &str) -> Option<&dyn Codec> {
    self.all_codecs().find(|codec| codec.id() == id)
  }

  pub fn detect(
    &self,
    data: &[u8],
  ) -> Result<Vec<(&dyn Codec, CodecHandlingConfidence)>, CodecError> {
    let mut codecs = self
      .all_codecs()
      .map(|codec| {
        debug!("{}", codec.id());
        Ok((codec, codec.can_handle(data)?))
      })
      .collect::<Result<Vec<_>, CodecError>>()?;

    codecs.retain(|(_, confidence)| *confidence != CodecHandlingConfidence::No);

    codecs.sort_by(|(codec_a, confidence_a), (codec_b, confidence_b)| {
      confidence_b
        .cmp(confidence_a) // DESC
        .then_with(|| codec_a.id().cmp(codec_b.id())) // ASC
    });

    Ok(codecs)
  }
}
