use crate::codecs::registry::CodecRegistry;
use mlua::Lua;
use mlua::LuaString;
use mlua::Table;
use sha2::Digest;
use sha2::Sha256;

use super::codec_handle::LuaCodecHandle;

fn get_codec(_: &Lua, id: String) -> Result<LuaCodecHandle, mlua::Error> {
  let codec = CodecRegistry::get_builtin(&id)
    .ok_or_else(|| mlua::Error::runtime(format!("Unknown builtin codec: {id}")))?;

  Ok(LuaCodecHandle { codec })
}

fn sha256(lua: &Lua, input: LuaString) -> Result<LuaString, mlua::Error> {
  let hash = hex::encode(Sha256::digest(input.as_bytes()));

  lua.create_string(&hash)
}

pub fn register_functions(lua: &Lua, ludared: &Table) -> Result<(), mlua::Error> {
  ludared.set("codec", lua.create_function(get_codec)?)?;
  ludared.set("sha256", lua.create_function(sha256)?)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  // TODO: This should go to mod.rs, and ensure that Lua::with_module() is
  // integrating the function.
  #[test]
  fn integrates_lua_function_for_sha256() -> Result<(), mlua::Error> {
    let lua = Lua::new();
    let ludared = lua.create_table()?;

    register_functions(&lua, &ludared)?;
    lua.globals().set("LUDARED", ludared)?;

    let hash: String = lua.load(r#"LUDARED.sha256("abc")"#).eval()?;

    assert_eq!(
      hash,
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );

    Ok(())
  }
}
