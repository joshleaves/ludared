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
  - [x] `create <PATH>` - Create a skeleton script file

- [x] `decode`
  - [x] `add <PATH> <CODEC> [ARGS] [--name NAME]` - Add a decode step
  - [x] `list <PATH>` - List decode steps on a node
  - [x] `remove <PATH> [NAME] [--clean]` - Remove a terminal decode step

- [x] `unpack` - Execute the manifest's full decode pipeline

- [x] `artifacts`
  - [x] `list [PATH] [--tree]` - List artifacts, optionally as a tree

- [x] `cache`
  - [x] `cat <PATH>` - Output a cached artifact's contents to STDOUT
  - [x] `path <PATH>` - Print a cached artifact's physical path

- [x] `override` - Manage the decoded artifacts materialized in the workspace
  - [x] `list [PATH] [--tree]` - Lists the artifacts declared as overrides, optionally as a tree
  - [x] `add <PATH> [--force]` - Declares an override and materializes the artifact in the workspace
  - [x] `refresh <PATH*>` - Rewrites an overridden artifact with its canonical bytes
  - [x] `remove <PATH> [--clean]` - Deactivates an override, optionally deleting its workspace file

- [x] `build [SOURCE]` - Run the manifest to produce build(s) from the source(s)
- [ ] `patch [SOURCE] [--format ips|bps|xdelta]` - Builds, and produces a patch against source 

- [x] `doctor` - Verify project configuration and source files
- [x] `clean` - Remove generated build and cache artifacts

- [x] `completions` - Generate completions for your shell

## Shell completions

ludared provides dynamic shell completions, including project-aware completion for sources and other project resources.

### Available globbing

You can use globbing on paths. The allowed patterns are:
- `?` - Single character match
- `*` - Sequence match
- `[a-z0-9-]` - Character class match
- `[a-z0-9-]+` - The same, but repeated
- `{aba,abc}` - Alternative string match

Globbing only triggers on those patterns, and the matching is done per path component, so no `source.bin/**/*.ttf`.

The commands allowing globs are:

- [x] `decode add <PATH*> <CODEC>`
- [x] `decode remove <PATH*> [NAME]`
- [x] `override add <PATH*>`
- [x] `override remove <PATH*>`
- [x] `override refresh <PATH*>` 
- [x] `codecs detect <PATH*>`

### Available completions
- [x] `codecs info <CODEC: completion_codecs_list>`
- [x] `codecs detect <PATH: completion_artifacts_list>`
- [x] `sources add <FILE: completion_sources_available>`
- [x] `sources remove <FILE: completion_sources_list>`
- [x] `decode add <PATH: completion_artifacts_decodable> <CODEC: completion_codecs_list>`
- [x] `decode remove <PATH: completion_artifacts_decoded> [NAME]`
- [x] `artifacts list [PATH: completion_artifacts_list]`
- [x] `cache path [PATH: completion_artifacts_list]`
- [x] `cache cat [PATH: completion_artifacts_list]`
- [x] `override list [PATH: completion_override_list]`
- [x] `override add <PATH: completion_override_addable>`
- [x] `override refresh <PATH: completion_override_active>`
- [x] `override remove <PATH: completion_override_active>`
- [x] `build [SOURCE: completion_sources_list]`
- [x] `patch [SOURCE: completion_sources_list]`

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
- [x] `std/sega/compression/prs` - Handles [PRS streams](https://segaretro.org/PRS_compression)

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
| `std/sega/compression/prs` | ❌ | ✅ |

**Note:** `N/A` means the property has not yet been verified, not that the codec lacks it. A codec must pass the corresponding [E2E tests](src/testing/codecs/e2e/mod.rs) before claiming either guarantee.

## Scripts codec

### Lua

> [!WARNING]
> Lua scripts are not sandboxed. They have access to the filesystem, environment variables, and other standard Lua facilities. Only run scripts from trusted sources.

For more information about Lua scripting in Ludared, see the [dedicated documentation](./SCRIPTS-LUA.md).

## To-do list

### Codecs
- [ ] Add versioning and metadata, with an ABI-friendly key/value metadata representation for future dynamic plugins.
- [ ] Add ruby & python. Maybe add .dll/.so/.dylib support before python. And add [malbolge](https://en.wikipedia.org/wiki/Malbolge) support before I add python.

### Configuration
- [ ] Reconsider build/cache path configuration: use `paths.builds` as the single configurable root for all disposable/generated data, with Ludared managing internal directories such as `cache/decodes` itself. Keep separate paths only if a concrete use case requires them (shared cache, separate storage, CI, etc.).

### Other stuff
- [ ] Migrate to [usage-rs](https://usage.jdx.dev/rust/migrating-from-clap#migrating-from-clap)
- [ ] Interface with [ratatui](https://ratatui.rs/installation/)
