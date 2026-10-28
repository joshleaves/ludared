use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::ScriptEnvironment;
use crate::codecs::cdrom::filesystem::iso9660::Iso9660;
use crate::codecs::cdrom::filesystem::udf::Udf;
use crate::codecs::cdrom::mode::mode1::Mode1;
use crate::codecs::cdrom::mode::mode2::Mode2;
use crate::codecs::cdrom::mode::mode2_xa::Mode2Xa;
use crate::codecs::errors::CodecError;
use crate::codecs::generic::extract::Extract;
use crate::codecs::generic::split::Split;
use crate::codecs::nintendo::n3ds::container::cci::N3dsCci;
use crate::codecs::nintendo::n3ds::container::cfa::N3dsCfa;
use crate::codecs::nintendo::n3ds::container::cia::N3dsCia;
use crate::codecs::nintendo::n3ds::container::cxi::N3dsCxi;
use crate::codecs::nintendo::n3ds::internal::exefs::N3dsExeFs;
use crate::codecs::nintendo::n3ds::internal::romfs::N3dsRomFs;
use crate::codecs::nintendo::snes::cart_exhirom::SnesExHiRom;
use crate::codecs::nintendo::snes::cart_hirom::SnesHiRom;
use crate::codecs::nintendo::snes::cart_lorom::SnesLoRom;
use crate::codecs::nintendo::snes::cart_sa1::SnesSa1Rom;
use crate::codecs::nintendo::snes::cart_sdd1::SnesSdd1Rom;
use crate::codecs::nintendo::snes::cart_spc7110::SnesSpc7110Rom;
use crate::codecs::scripts::lua::LuaCodec;
use crate::codecs::sega::compression::prs::PrsCompression;
use crate::codecs::sony::playstation::graphics::str_xa::StrXa;
use crate::scripts::discovery::ScriptDiscovery;
use crate::scripts::discovery::ScriptKind;
use crate::scripts::errors::ScriptError;
use log::*;
use std::path::PathBuf;

static BUILTINS: &[&dyn Codec] = &[
  &N3dsCci,
  &N3dsCia,
  &N3dsCfa,
  &N3dsCxi,
  &N3dsExeFs,
  &N3dsRomFs,
  &Extract,
  &Iso9660,
  &Mode1,
  &Mode2,
  &Mode2Xa,
  &Udf,
  &Split,
  &SnesExHiRom,
  &SnesLoRom,
  &SnesHiRom,
  &SnesSa1Rom,
  &SnesSdd1Rom,
  &SnesSpc7110Rom,
  &PrsCompression,
  &StrXa,
];

pub(crate) struct CodecRegistry {
  scripts_root: PathBuf,
  scripts: Vec<Box<dyn Codec>>,
}

impl CodecRegistry {
  pub fn new(scripts_root: PathBuf, env: ScriptEnvironment) -> Result<Self, ScriptError> {
    let mut registry = Self {
      scripts_root,
      scripts: Vec::new(),
    };

    registry.discover_scripts(env)?;

    Ok(registry)
  }

  fn discover_scripts(&mut self, env: ScriptEnvironment) -> Result<(), ScriptError> {
    let scripts = ScriptDiscovery::new(self.scripts_root.clone())?;

    // TODO: Script setup should belong to each Script codec
    // Prepare Lua VM
    let lua = mlua::Lua::new();
    let ludared = lua.create_table()?;
    for (key, value) in &env.0 {
      ludared.set(key.as_str(), value.as_str())?;
    }
    crate::codecs::scripts::lua::functions::register_functions(&lua, &ludared)?;
    lua.globals().set("LUDARED", ludared)?;

    for script_path in scripts.retrieve_scripts(ScriptKind::Codec) {
      if script_path.extension() != Some(std::ffi::OsStr::new("lua")) {
        continue;
      }
      let path = self.scripts_root.join(script_path);
      let id = format!("scripts/{}", script_path.to_string_lossy());

      let codec = LuaCodec::new(&lua, path, id, env.clone())?;
      self.scripts.push(Box::new(codec));
    }

    Ok(())
  }

  pub fn all_codecs(&self) -> impl Iterator<Item = &dyn Codec> {
    BUILTINS
      .iter()
      .copied()
      .chain(self.scripts.iter().map(|codec| codec.as_ref()))
  }

  pub fn get_builtin(id: &str) -> Option<&'static dyn Codec> {
    BUILTINS.iter().copied().find(|codec| codec.id() == id)
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
