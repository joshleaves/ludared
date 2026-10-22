# ludared

ludared (LUDus ARchive EDitor) is a command-line tool to help build and maintain ROM hacking and game modding projects.

**NOTE**: This project uses LLMs (AI), you can find out how in [AI_USAGE.md](./AI_USAGE.md), written exclusively by a human (and it shows).

# Current status
> **Status:** Kinda usable actually, the [Sakura Taisen FR romhack project](https://github.com/sakurayaku/re.sakurataisen) just published [their first beta](https://github.com/sakurayaku/re.sakurataisen/releases/tag/b20261018).

## Commands

- [x] `init` - Set up a project

- [x] `sources`
  - [x] `list` - List configured source files
  - [x] `add` - Add a source file to the manifest
  - [x] `remove` - Remove a source file from the manifest

- [x] `codecs`
  - [x] `list [--tree]` - List available codecs, optionally as a tree
  - [x] `info` - Get information about a specific codec
  - [x] `detect` - Detect which codecs can be used on a file

- [x] `decode`
  - [x] `add <PATH> <CODEC> [ARGS] [--name NAME]` - Add a decode step
  - [ ] `list <PATH>` - List decode steps on a node
  - [ ] `remove <PATH> [NAME]` - Remove a decode step

- [x] `unpack` - Execute the manifest's full decode pipeline

- [x] `artifacts`
  - [x] `list [VPATH] [--tree]` - List artifacts, optionally as a tree

- [x] `cache`
  - [x] `cat <VPATH>` - Output a cached artifact's contents to STDOUT
  - [x] `path <VPATH>` - Print a cached artifact's physical path

- [x] `override` - Manage the decoded artifacts materialized in the workspace
  - [x] `list [VPATH] [--tree]` - Lists the artifacts declared as overrides, optionally as a tree
  - [x] `add <VPATH> [--force]` - Declares an override and materializes the artifact in the workspace
  - [x] `refresh <VPATH>` - Rewrites an overridden artifact with its canonical bytes
  - [x] `remove <VPATH> [--clean]` - Deactivates an override, optionally deleting its workspace file

- [x] `build` - Run the manifest to produce build(s) from the source(s)

- [x] `doctor` - Verify project configuration and source files
- [x] `clean` - Remove generated build and cache artifacts

- [ ] `patch [--format ips|bps|xdelta]` - Builds, and produces a patch against source 

- [x] `completions` - Generate completions for your shell

## Shell completions

ludared provides dynamic shell completions, including project-aware completion for sources and other project resources.

Use `ludared completions [SHELL] | source`. If you don't provide a shell, it will be identified best-effort from your `$SHELL` environment variable.

### Available completions
- [x] `codecs info <CODEC: complete_codecs_list>`
- [x] `sources add <FILE: complete_source_add>`
- [x] `sources remove <FILE: complete_source_remove>`
- [x] `decode add <PATH: complete_artifacts_decodable> <CODEC: complete_codecs_list>`
- [x] `artifacts list [PATH: complete_artifacts_listing]`
- [x] `cache path [PATH: complete_artifacts_listing]`
- [x] `cache cat [PATH: complete_artifacts_listing]`
- [x] `override list [PATH: complete_override_listing]`
- [x] `override add <PATH: complete_override_addable>`
- [x] `override refresh <PATH: complete_override_active>`
- [x] `override remove <PATH: complete_override_active>`
- [x] `build [SOURCE: complete_source_remove]`

## Available codecs

Codecs come in two types: `Rebuild` and `Patch`. Simply put [one is archival, the other is byte replacement](the_two_types_of_codecs.jpeg).

### Rebuild codecs
A `Rebuild` codec consumes ALL of its input, so it is the only _decode_ that can be ran on an artifact.

- [x] `std/generic/split` - Turn a single blob into mulitple blobs
- [x] `std/nintendo/3ds/container/cci` - Split a Nintendo 3DS CCI (NCSD) cartridge image into its NCCH partitions
- [x] `std/nintendo/snes/cart/exhirom`- Cut a SNES ExHiROM into ROM banks
- [x] `std/nintendo/snes/cart/hirom`- Cut a SNES HiROM into ROM banks
- [x] `std/nintendo/snes/cart/lorom`- Cut a SNES LoROM into ROM banks
- [x] `std/nintendo/snes/cart/sa1`- Cut a SNES SA-1 ROM into ROM banks
- [x] `std/nintendo/snes/cart/sdd1`- Cut a SNES SDD-1 ROM into ROM banks
- [x] `std/nintendo/snes/cart/spc7110`- Cut a SNES SPC7110 ROM into ROM banks
- [x] `std/cdrom/iso9660` - Extract the filesystem of an ISO 9660 CD-ROM image
- [x] `std/cdrom/mode2352` - Turn tracks with 2352-byte MODE1 sectors into a single ISO image artifact

### Patch codecs
Multiple `Patch` codecs can be ran against a single artefact, as each will take a part. The ludared engine guarantees they are always ran in the manifest definition order.

- [x] `std/generic/extract` - Extract arbitrary bytes from a blob

### Content preservation

All codecs **MUST** support round-tripping: they can `decode` content and `encode` it back. Some codecs provide additional guarantees:

- **BE (Bit-exact):** Decoding a valid input and re-encoding the resulting artifacts without modifications (and with the same arguments) reproduces the original input byte-for-byte.
- **Det. (Deterministic):** Given identical inputs and arguments, `decode` and `encode` always produce identical results.

| Plugin | BE | Det. |
|---|:---:|:---:|
| `std/generic/split` | ✅ | ✅ |
| `std/generic/extract` | ✅ | ✅ |
| `std/nintendo/3ds/container/cci` | ✅ | ✅ |
| `std/nintendo/snes/cart/exhirom` | N/A | N/A |
| `std/nintendo/snes/cart/hirom` | N/A | N/A |
| `std/nintendo/snes/cart/lorom` | N/A | N/A |
| `std/nintendo/snes/cart/sa1` | N/A | N/A |
| `std/nintendo/snes/cart/sdd1` | N/A | N/A |
| `std/nintendo/snes/cart/spc7110` | N/A | N/A |
| `std/cdrom/iso9660` | ✅ | ✅ |
| `std/cdrom/mode2352` | ✅ | ✅ |

**Note:** `N/A` means the property has not yet been verified, not that the codec lacks it. A codec must pass the corresponding [E2E tests](src/testing/codecs/e2e/mod.rs) before claiming either guarantee.

## Scripts codec

### Lua

Lua is embedded through [mlua](https://github.com/mlua-rs/mlua), with Lua 5.5 support.

Scripts have access to the following project-specific globals:

```lua
local workspace = LUDARED.WORKSPACE
local project_root = LUDARED.PROJECT_ROOT
local scripts = LUDARED.SCRIPTS
```

Ludared also exposes the following utility functions:

```lua
local hash = LUDARED.sha256(data) -- SHA-256 digest as a lowercase hexadecimal string
```

> [!WARNING]
> Lua scripts are not sandboxed. They have access to the filesystem, environment variables, and other standard Lua facilities. Only run scripts from trusted sources.

A Lua codec must return a table with the following fields:

```lua
return {
  name = "My Codec",
  description = "Description of the codec",
  version = 1,
  kind = "patch", -- "patch" or "rebuild"

  can_handle = function(data)
    return "no" -- "no", "possible", "likely", "certain"
  end,

  decode_name = function(args)
    return "output.bin"
  end,

  decode = function(data, args)
    return {
        { name = "output.bin", data = data },
        { name = "metadata.bin", data = metadata },
    }
  end,

  encode = function(original, artifacts, args)
    return artifacts["input.bin"]
  end,
}
```

## To-do list

### Codecs
- [ ] Add versioning and metadata, with an ABI-friendly key/value metadata representation for future dynamic plugins.

### Configuration
- [ ] Reconsider build/cache path configuration: use `paths.builds` as the single configurable root for all disposable/generated data, with Ludared managing internal directories such as `cache/decodes` itself. Keep separate paths only if a concrete use case requires them (shared cache, separate storage, CI, etc.).

### Other stuff
- [ ] Migrate to [usage-rs](https://usage.jdx.dev/rust/migrating-from-clap#migrating-from-clap)
- [ ] Interface with [ratatui](https://ratatui.rs/installation/)
