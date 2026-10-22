# Ludared Lua codec API

Technical reference for Lua codec authors. For architectural rules, workflow, confidence semantics, and project-file read policy, see [../SKILL.md](../SKILL.md).

> This reference is reconstructed from the API details established in our discussion, not verified against the current Rust source. Check the actual `mlua` binding and project codecs before relying on exact signatures or runtime error wording.

## Runtime

Ludared embeds **Lua 5.5 via mlua**. Lua 5.3+ binary facilities are available, including `string.pack`, `string.unpack`, `string.packsize`, integer arithmetic, and bitwise operators (`&`, `|`, `~`, `<<`, `>>`). Lua strings can hold arbitrary binary bytes, including NUL. Check the runtime before assuming optional native modules (`lsqlite3`, JSON libraries, etc.) are installed.

Each codec invocation uses a fresh Lua VM; globals do not persist between detection, decode, and encode. Modules are loaded eagerly during codec discovery.

## Module shape

A codec script returns a Lua table with these fields:

```lua
return {
  name = "Example codec",               -- display name
  description = "Describe the format",  -- human-readable description
  version = 1,
  kind = "rebuild",                     -- or "patch"

  can_handle = function(data)
    return "likely"                     -- no / possible / likely / certain
  end,

  decode_name = function(args)
    return "decoded"
  end,

  decode = function(data, args)
    return { { name = "part.bin", data = data } }
  end,

  encode = function(original, artifacts, args)
    return artifacts[1].data
  end,
}
```

The sample demonstrates **shape only**, not format recognition or a production-ready codec. `kind` is a reconstruction strategy: `rebuild` creates the complete parent and receives an empty `original`; `patch` starts from the parent bytes. Multiple patches on a node are ordered; do not mix patch and rebuild on the same node.

## Callback values

| Callback | Inputs | Return |
| --- | --- | --- |
| `can_handle(data)` | Binary input string | `"no"`, `"possible"`, `"likely"`, `"certain"` |
| `decode_name(args)` | Codec-owned opaque argument string or `nil` | Output node name string |
| `decode(data, args)` | Binary input string, arguments | Ordered array of `{name = string, data = binary_string}` |
| `encode(original, artifacts, args)` | Parent bytes, ordered artifacts, arguments | Reconstructed binary string |

`decode` must return an **array of records**, not a table keyed by artifact names:

```lua
return {
  { name = "head.bin", data = head },
  { name = "tail.bin", data = tail },
}
```

Output names are relative virtual paths, and their order is retained for `encode`. Validate names, counts, sizes, and format constraints when reconstructing; workspace overrides can change the contents.

Arguments belong to the codec: Ludared records and passes an opaque string (or `nil`), but does not impose a `key=value` syntax or JSON encoding.

## Project paths and module loading

`LUDARED.WORKSPACE` identifies the workspace directory. `LUDARED.SCRIPTS` identifies the scripts directory. They are filesystem paths, not virtual-artifact read APIs. For example:

```lua
local helper = dofile(LUDARED.SCRIPTS .. "/shared/helper.lua")
local f = assert(io.open(LUDARED.WORKSPACE .. "/tables/glyphs.tbl", "rb"))
local contents = f:read("*a")
f:close()
```

`require` does **not** automatically include the scripts directory in `package.path`; configure module lookup explicitly if needed. Reading project-specific resources (lookup tables, read-only databases, files from another branch) is supported by the intended Lua workflow, but such dependencies may not be tracked by the engine. Keep them stable and document them. In particular, do not assume a source file is present as a workspace override during `decode`; use known pristine reference data when appropriate. **Do not write to the workspace during Ludared operations.** Independent branches have no guaranteed execution order.

## Discovery

Script IDs are paths relative to the configured `paths.scripts` root, including `.lua` (for example `scripts/my_codec.lua` in CLI usage). Every `*.lua` file beneath that root is a codec candidate **except** files in directories excluded by discovery. The currently documented exclusions are `shared/` and `libraries/`; `tests/` is a proposed organizational improvement, **not a verified exclusion**. Verify discovery rules in the current source before placing test helpers beneath the scripts root.

## Binary parsing example

```lua
local value, next_offset = string.unpack("<I4", data, 1) -- unsigned 32-bit LE
local first_byte = string.byte(data, next_offset)
local flags = value & 0xFF
local packed = string.pack("<I4", value)
```

Lua string positions are 1-based; `string.unpack` returns the next position. Validate bounds and integer ranges for untrusted binary inputs.

## Error behavior

Use descriptive Lua errors for malformed inputs and invalid reconstruction artifacts. A failed module load may prevent project initialization because discovery eagerly loads codec scripts. For exact engine-specific error messages, consult the current Rust bindings rather than assuming stable text.
