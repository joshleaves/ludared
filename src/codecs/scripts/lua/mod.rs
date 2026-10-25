use crate::codecs::Codec;
use crate::codecs::CodecHandlingConfidence;
use crate::codecs::CodecKind;
use crate::codecs::DecodedArtifact;
use crate::codecs::ScriptEnvironment;
use crate::codecs::errors::CodecError;
use crate::scripts::errors::ScriptError;
use std::path::PathBuf;

pub const SKELETON: &str = include_str!("./skeleton.lua");

mod codec_handle;
pub mod functions;

const REQUIRED_FUNCTIONS: [&str; 4] = [
  "can_handle",
  "decode_name",
  "decode",
  "encode",
];

pub(crate) struct LuaCodec {
  id: String,
  path: PathBuf,
  name: String,
  kind: CodecKind,
  description: String,
  #[allow(dead_code)]
  version: u32,
  environment: ScriptEnvironment,
}

impl LuaCodec {
  pub fn new(
    lua: &mlua::Lua,
    path: PathBuf,
    id: String,
    environment: ScriptEnvironment,
  ) -> Result<Self, ScriptError> {
    let source = std::fs::read(&path)?;
    let module: mlua::Table = lua.load(&source).set_name(path.to_string_lossy()).eval()?;

    let name: String = module.get("name")?;
    let kind: String = module.get::<String>("kind")?;
    let kind: CodecKind = match kind.as_str() {
      "rebuild" => CodecKind::Rebuild,
      "patch" => CodecKind::Patch,
      _ => return Err(ScriptError::InvalidCodecKind(kind.clone())),
    };
    let description: String = module.get("description")?;
    let version: u32 = module.get("version")?;

    // All functions are mandatory.
    for name in REQUIRED_FUNCTIONS {
      let _: mlua::Function = module
        .get(name)
        .map_err(|_| ScriptError::InvalidCodecFunction(name.into()))?;
    }

    Ok(Self {
      id,
      path,
      name,
      kind,
      description,
      version,
      environment,
    })
  }

  /// Hands `call` the codec's module, evaluated in a Lua VM of its own.
  ///
  /// VMs are ephemeral. Discovery uses one to read the module's metadata out, and
  /// every operation opens its own, because a VM is neither `Send` nor `Sync`
  /// while every codec must be both. Nothing of the script survives between
  /// operations either: it is read and evaluated again each time, and no bytecode
  /// is kept.
  fn with_module<T>(
    &self,
    call: impl FnOnce(&mlua::Lua, &mlua::Table) -> Result<T, ScriptError>,
  ) -> Result<T, CodecError> {
    let lua = mlua::Lua::new();
    let ludared = lua.create_table()?;
    for (key, value) in &self.environment.0 {
      ludared.set(key.as_str(), value.as_str())?;
    }
    functions::register_functions(&lua, &ludared)?;
    lua.globals().set("LUDARED", ludared)?;

    let source = std::fs::read(&self.path)?;
    let module: mlua::Table = lua
      .load(&source)
      .set_name(self.path.to_string_lossy())
      .eval()
      .map_err(ScriptError::from)?;

    Ok(call(&lua, &module)?)
  }

  /// The named entry point of the module.
  ///
  /// The constructor proved every entry point is there, so this only fails if the
  /// file was rewritten underneath the codec between discovery and the call.
  fn entry(module: &mlua::Table, name: &str) -> Result<mlua::Function, ScriptError> {
    module
      .get(name)
      .map_err(|_| ScriptError::InvalidCodecFunction(name.into()))
  }

  /// The codec arguments as its script sees them: the string it was handed, or
  /// nothing at all when there were none.
  ///
  /// Arguments are opaque here, exactly as `Codec` leaves them. What they mean is
  /// the codec's own to say, whether that is JSON, TOML, a bare number, or
  /// anything else, so nothing about them is parsed, validated, or converted.
  fn args(lua: &mlua::Lua, args: Option<&str>) -> Result<mlua::Value, ScriptError> {
    Ok(match args {
      Some(args) => mlua::Value::String(lua.create_string(args)?),
      None => mlua::Value::Nil,
    })
  }

  /// The artifacts a decode listed, in the order it listed them.
  ///
  /// Order is the codec's to choose and the manifest's to preserve, so the array
  /// is read in sequence rather than through `pairs`, whose walk order Lua leaves
  /// undefined. Bytes ride as a Lua string, which is a byte string and loses
  /// nothing on the way back.
  fn artifacts(artifacts: &mlua::Table) -> Result<Vec<DecodedArtifact>, ScriptError> {
    let mut decoded = Vec::new();

    for artifact in artifacts.sequence_values::<mlua::Table>() {
      let artifact = artifact?;

      decoded.push(DecodedArtifact {
        name: artifact.get("name")?,
        data: artifact.get::<mlua::LuaString>("data")?.as_bytes().to_vec(),
      });
    }

    Ok(decoded)
  }

  /// The artifacts of an encode, laid out the way a decode lists them.
  fn artifact_table(
    lua: &mlua::Lua,
    artifacts: &[DecodedArtifact],
  ) -> Result<mlua::Table, ScriptError> {
    let table = lua.create_table_with_capacity(artifacts.len(), 0)?;

    for (index, artifact) in artifacts.iter().enumerate() {
      let record = lua.create_table_with_capacity(0, 2)?;

      record.set("name", artifact.name.as_str())?;
      record.set("data", lua.create_string(&artifact.data)?)?;
      // Lua arrays are one-based.
      table.set(index + 1, record)?;
    }

    Ok(table)
  }
}

impl Codec for LuaCodec {
  fn id(&self) -> &str {
    &self.id
  }

  fn name(&self) -> &str {
    &self.name
  }

  fn description(&self) -> &str {
    &self.description
  }

  fn kind(&self) -> CodecKind {
    self.kind
  }

  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError> {
    let handled: String = self.with_module(|lua, module| {
      let can_handle = Self::entry(module, "can_handle")?;
      Ok(can_handle.call(lua.create_string(data)?)?)
    })?;

    match handled.as_str() {
      "no" => Ok(CodecHandlingConfidence::No),
      "possible" => Ok(CodecHandlingConfidence::Possible),
      "likely" => Ok(CodecHandlingConfidence::Likely),
      "certain" => Ok(CodecHandlingConfidence::Certain),
      _ => Err(ScriptError::InvalidCodecConfidence(handled).into()),
    }
  }

  fn decode_name(&self, args: Option<&str>) -> Result<String, CodecError> {
    self.with_module(|lua, module| {
      let decode_name = Self::entry(module, "decode_name")?;

      Ok(decode_name.call(Self::args(lua, args)?)?)
    })
  }

  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError> {
    self.with_module(|lua, module| {
      let decode = Self::entry(module, "decode")?;
      let data = lua.create_string(data)?;
      let args = Self::args(lua, args)?;

      Self::artifacts(&decode.call::<mlua::Table>((data, args))?)
    })
  }

  fn encode(
    &self,
    original: Vec<u8>,
    artifacts: &[DecodedArtifact],
    args: Option<&str>,
  ) -> Result<Vec<u8>, CodecError> {
    self.with_module(|lua, module| {
      let encode = Self::entry(module, "encode")?;
      let original = lua.create_string(&original)?;
      let artifacts = Self::artifact_table(lua, artifacts)?;
      let args = Self::args(lua, args)?;

      let encoded = encode.call::<mlua::LuaString>((original, artifacts, args))?;

      Ok(encoded.as_bytes().to_vec())
    })
  }
}
