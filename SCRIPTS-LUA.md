# Lua scripting

> [!WARNING]
> Lua scripts are not sandboxed. They have access to the filesystem, environment variables, and other standard Lua facilities. Only run scripts from trusted sources.

Ludared embeds [Lua 5.5](https://www.lua.org/) through [mLua](https://github.com/mlua-rs/mlua). Lua scripts can implement codecs and call Ludared's builtin codecs.

## Writing Lua codecs

> [!NOTE]
> You can generate a skeleton script with `ludared codecs create <script_name>.lua`.

A Lua codec script returns a table describing the codec and implementing its operations:

```lua
return {
  name = "My Codec",
  description = "Description of the codec",
  version = 1,
  -- Codec kind:
  -- - "rebuild" reconstructs the entire file from scratch
  -- - "patch" is passed the original file and applies modifications
  kind = "rebuild",

  --- Determines whether this codec can handle the input.
  --- Possible values:
  --- - `"no"`: "I cannot decode that data meaningfully, no way."
  --- - `"possible"`: "I may extract something out of that data, maybe garbage, maybe gold."
  --- - `"likely"`: "All signs point to yes, but false positives are possible."
  --- - `"certain"`: "I know that data, I know that format, I'm in charge."
  ---@param data string Input binary data.
  ---@return "no"|"possible"|"likely"|"certain"
  can_handle = function(data)
    return "no"
  end,

  --- Generates a name for the decode operation.
  ---@param args string? Codec arguments.
  ---@return string
  decode_name = function(args)
    return "codec"
  end,

  --- Extracts artifacts from the input.
  ---@param data string Input binary data.
  ---@param args string? Codec arguments.
  ---@return { name: string, data: string }[]
  decode = function(data, args)
    return {
      { name = "output.bin", data = data },
      { name = "metadata.bin", data = "" },
    }
  end,

  --- Reconstructs the original file from its artifacts.
  ---@param original string Original bytes (empty for rebuild codecs).
  ---@param artifacts { name: string, data: string }[] Ordered artifacts.
  ---@param args string? Codec arguments.
  ---@return string
  encode = function(original, artifacts, args)
    return artifacts[1].data
  end,
}
```

The example is a template illustrating the interface; adapt the decoding, encoding, and artifact names to your format.

### Codec metadata

| Field | Description |
| --- | --- |
| `name` | Human-readable codec name. |
| `description` | Short description of the format or transformation. |
| `version` | Codec version number. |
| `kind` | `"patch"` or `"rebuild"`. |

### Codec callbacks

| Callback | Purpose |
| --- | --- |
| `can_handle(data)` | Return `"no"`, `"possible"`, `"likely"`, or `"certain"` to indicate recognition confidence. |
| `decode_name(args)` | Suggest a name for a decode operation. |
| `decode(data, args)` | Return an array of `{ name = ..., data = ... }` artifacts. |
| `encode(original, artifacts, args)` | Return the reconstructed binary data. |

`args` contains the codec-specific configuration, when supplied. `decode_name` identifies the decode operation, not an individual output artifact.

## Script environment

Ludared provides a `LUDARED` global with project-specific paths:

```lua
local workspace = LUDARED.WORKSPACE
local project_root = LUDARED.PROJECT_ROOT
local scripts = LUDARED.SCRIPTS
```

These values are supplied by Ludared for the current project and script environment.

## Utility functions

```lua
local hash = LUDARED.sha256(data)
local codec = LUDARED.codec(codec_id)
```

- **`LUDARED.sha256(data)`** returns the SHA-256 digest of a binary string as lowercase hexadecimal text.
- **`LUDARED.codec(codec_id)`** returns a handle to a registered builtin codec. An unknown builtin codec ID raises a Lua error.

Lua strings can contain arbitrary binary data, including null bytes.

## Using builtin codecs

A builtin codec handle exposes read-only properties and methods.

```lua
local codec = LUDARED.codec("std/sega/compression/prs")

print(codec.id)
print(codec.name)
print(codec.description)
print(codec.kind)
```

### Properties

| Property | Description |
| --- | --- |
| `codec.id` | Registered builtin codec ID. |
| `codec.name` | Human-readable name. |
| `codec.description` | Description of the codec. |
| `codec.kind` | Encoding strategy: `"patch"` or `"rebuild"`. |

### Methods

**`codec:can_handle(data)`** checks whether the codec recognizes the binary input. It returns one of `"no"`, `"possible"`, `"likely"`, or `"certain"`, in increasing order of confidence. Recognition does not modify the input or produce artifacts.

**`codec:decode(data[, args])`** decodes a binary string into a Lua sequence of artifacts. Each artifact is a table with `name` (string) and `data` (binary string) fields.

```lua
local artifacts = codec:decode(data)
for _, artifact in ipairs(artifacts) do
  print(artifact.name, #artifact.data)
end
```

**`codec:encode(original, artifacts[, args])`** encodes a sequence of artifact tables into a binary string. For a `"rebuild"` codec, pass `""` as `original`; for a `"patch"` codec, pass the original input bytes.

```lua
local encoded = codec:encode(
  codec.kind == "patch" and data or "",
  artifacts
)
```

The optional `args` parameter is a codec-specific configuration string. Use consistent arguments when decoding and encoding the same data.

## Testing Lua codecs

Ludared now comes with a `ludared scripts test [PATH*]` command. It will discover test scripts in your configured `scripts` folder, skip all `shared` folders, and collect files under `tests` folders. Any `.lua` file under a test directory is fair game, so a test fixture should go in `tests/shared/fixture.lua` (or even `tests/namespace/feature/that-object/shared/fixture.lua`) to avoid being picked up.

There is no specific test harness. You are free to run your tests using `assert()` and `print()`. I haven't tested [LuaUnit](https://github.com/bluebird75/luaunit) or [busted](https://github.com/lunarmodules/busted), but they may work with some additional setup (feedback welcome).
