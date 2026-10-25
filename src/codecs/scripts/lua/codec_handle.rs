use crate::codecs::Codec;
use crate::codecs::DecodedArtifact;
use mlua::LuaString;
use mlua::Table;
use mlua::UserData;
use mlua::UserDataFields;
use mlua::UserDataMethods;

pub(super) struct LuaCodecHandle {
  pub(super) codec: &'static dyn Codec,
}

impl UserData for LuaCodecHandle {
  fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
    fields.add_field_method_get("id", |_, this| Ok(this.codec.id()));
    fields.add_field_method_get("name", |_, this| Ok(this.codec.name()));
    fields.add_field_method_get("description", |_, this| Ok(this.codec.description()));
    fields.add_field_method_get("kind", |_, this| Ok(this.codec.kind().to_string().to_lowercase()));
  }

  fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
    methods.add_method("can_handle", |_, this, data: LuaString| {
      let compatibility = this
        .codec
        .can_handle(data.as_bytes().as_ref())
        .map_err(mlua::Error::external)?;
      Ok(compatibility.to_string().to_lowercase())
    });

    methods.add_method(
      "decode",
      |lua, this, (data, args): (LuaString, Option<String>)| {
        let artifacts = this
          .codec
          .decode(data.as_bytes().as_ref(), args.as_deref())
          .map_err(mlua::Error::external)?;

        let result = lua.create_table()?;

        for (i, artifact) in artifacts.into_iter().enumerate() {
          let entry = lua.create_table()?;

          entry.set("name", artifact.name)?;
          entry.set("data", lua.create_string(&artifact.data)?)?;

          result.set(i + 1, entry)?;
        }

        Ok(result)
      },
    );

    methods.add_method(
      "encode",
      |lua, this, (original, artifacts, args): (LuaString, Table, Option<String>)| {
        let mut decoded = Vec::new();

        for entry in artifacts.sequence_values::<Table>() {
          let entry = entry?;

          let name: String = entry.get("name")?;
          let data: LuaString = entry.get("data")?;

          decoded.push(DecodedArtifact {
            name,
            data: data.as_bytes().to_vec(),
          });
        }

        let encoded = this
          .codec
          .encode(original.as_bytes().to_vec(), &decoded, args.as_deref())
          .map_err(mlua::Error::external)?;

        Ok(lua.create_string(&encoded))
      },
    );
  }
}
