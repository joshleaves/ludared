use mlua::Lua;
use mlua::LuaString;
use mlua::Result;
use mlua::Table;
use sha2::Digest;
use sha2::Sha256;

fn sha256(lua: &Lua, input: LuaString) -> Result<LuaString> {
  let hash = hex::encode(Sha256::digest(input.as_bytes()));

  lua.create_string(&hash)
}

pub(super) fn register_functions(lua: &Lua, ludared: &Table) -> Result<()> {
  ludared.set("sha256", lua.create_function(sha256)?)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  // TODO: This should go to mod.rs, and ensure that Lua::with_module() is
  // integrating the function.
  #[test]
  fn integrates_lua_function_for_sha256() -> Result<()> {
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
