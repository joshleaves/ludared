# ludared

ludared (LUDus ARchive EDitor) is a command-line tool to help build and maintain ROM hacking and game modding projects.

# Current status
> **Status:** Kind usable actually.

## Available commands

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
  - [x] `add` - Add a decode step

- [x] `unpack` - Execute the manifest's decode pipeline

- [x] `artifacts`
  - [x] `list [VPATH] [--tree]` - List artifacts, optionally as a tree

- [x] `cache`
  - [x] `cat` - Output a cached artifact's contents to STDOUT
  - [x] `path` - Print a cached artifact's physical path

- [x] `override` - Manage the decoded artifacts materialized in the workspace
  - [x] `list [VPATH] [--tree]` - Lists the artifacts declared as overrides, optionally as a tree
  - [x] `add <VPATH> [--force]` - Declares an override and materializes the artifact in the workspace
  - [x] `refresh <VPATH>` - Rewrites an overridden artifact with its canonical bytes
  - [x] `remove <VPATH> [--clean]` - Deactivates an override, optionally deleting its workspace file

- [x] `build` - Run the manifest to produce build(s) from the source(s)

- [x] `doctor` - Verify project configuration and source files
- [x] `clean` - Remove generated build and cache artifacts

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
- [x] `override list [PATH: complete_override_listing]`
- [x] `override add <PATH: complete_override_addable>`
- [x] `override refresh <PATH: complete_override_active>`
- [x] `override remove <PATH: complete_override_active>`
- [x] `build [SOURCE: complete_source_remove]`

## Available codecs

Codecs come in two types: `Rebuild` and `Patch`. Simply put [one is archival, the other is replacement](there_are_two_codec_types.png).

### Rebuild codecs
A `Rebuild` codec consumes ALL of its input, so it is the only _decode_ that can be ran on an artifact.

- [x] `std/generic/split` - Turn a single blob into mulitple blobs
- [x] `std/nintendo/snes/cart/exhirom`- Cut a SNES ExHiROM into ROM banks
- [x] `std/nintendo/snes/cart/hirom`- Cut a SNES HiROM into ROM banks
- [x] `std/nintendo/snes/cart/lorom`- Cut a SNES LoROM into ROM banks
- [x] `std/nintendo/snes/cart/sa1`- Cut a SNES SA-1 ROM into ROM banks
- [x] `std/nintendo/snes/cart/sdd1`- Cut a SNES SDD-1 ROM into ROM banks
- [x] `std/nintendo/snes/cart/spc7110`- Cut a SNES SPC7110 ROM into ROM banks
- [x] `std/cdrom/iso9660` - Extract the filesystem of an ISO 9660 CD-ROM image. Uses [hadris-iso](https://github.com/hxyulin/hadris).
- [x] `std/cdrom/mode2352` - Turn tracks with 2352-byte MODE1 sectors into a single ISO image artifact

### Patch codecs
Multiple `Patch` codecs can be ran against a single artefact, as each will take a part. The ludared engine guarantees they are always ran in the manifest definition order.

- [x] `std/generic/extract` - Extract arbitrary bytes from a blob

## To-do list

### Planned commands

- [ ] `decode`
  - [ ] `list` - `decode list [VPATH]`
  - [ ] `remove` - `decode remove <VPATH> <NAME> `

- [ ] `archive`
- [ ] `tool`

### Codecs
- [ ] Add versioning and metadata, with an ABI-friendly key/value metadata representation for future dynamic plugins.

### Configuration
- [ ] Reconsider build/cache path configuration: use `paths.builds` as the single configurable root for all disposable/generated data, with Ludared managing internal directories such as `cache/decodes` itself. Keep separate paths only if a concrete use case requires them (shared cache, separate storage, CI, etc.).

### Other stuff
- [ ] Migrate to [usage-rs](https://usage.jdx.dev/rust/migrating-from-clap#migrating-from-clap)
- [ ] Interace with [ratatui](https://ratatui.rs/installation/)
