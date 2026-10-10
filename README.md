# ludared

ludared (LUDus ARchive EDitor) is a command-line tool to help build and maintain ROM hacking and game modding projects.

# Current status
> **Status:** Early development.

## Available commands

- [x] `init` - Set up a project

- [x] `sources`
  - [x] `list` - List configured source files
  - [x] `add` - Add a source file to the manifest
  - [x] `remove` - Remove a source file from the manifest

- [x] `codecs`
  - [x] `list` - List available codecs
  - [x] `info` - Get information about a specific codec
  - [x] `detect` - Detect which codecs can be used on a file

- [x] `decode`
  - [x] `add` - Add a decode step

- [x] `unpack` - Execute the manifest's decode pipeline

- [x] `artifacts`
  - [x] `list [VPATH]` - List artifacts
  - [x] `tree [VPATH]` - Display artifacts as a tree

- [x] `cache`
  - [x] `cat` - Output a cached artifact's contents to STDOUT
  - [x] `path` - Print a cached artifact's physical path

- [x] `override` - Manage the decoded artifacts materialized in the workspace
  - [x] `list [VPATH]` - Lists the artifacts declared as overrides
  - [x] `tree [VPATH]` - Displays declared overrides as a tree
  - [x] `add <VPATH> [--force]` - Declares an override and materializes the artifact in the workspace
  - [x] `refresh <VPATH>` - Rewrites an overridden artifact with its canonical bytes
  - [x] `remove <VPATH> [--clean]` - Deactivates an override, optionally deleting its workspace file

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
- [x] `artifacts tree [PATH: complete_artifacts_listing]`
- [x] `override list [PATH: complete_override_listing]`
- [x] `override tree [PATH: complete_override_listing]`
- [x] `override add <PATH: complete_override_addable>`
- [x] `override refresh <PATH: complete_override_active>`
- [x] `override remove <PATH: complete_override_active>`

## Planned commands

- [ ] `decode`
  - [x] `add` - `decode add <VPATH> <CODEC> [ARGS] [NAME]`
  - [ ] `list` - `decode list [VPATH]`
  - [ ] `remove` - `decode remove <VPATH> <NAME> `

- [ ] `build`

- [ ] `archive`
- [ ] `tool`

## To-do list

### Codecs
- [ ] Add versioning and metadata, with an ABI-friendly key/value metadata representation for future dynamic plugins.

### Configuration
- [ ] Reconsider build/cache path configuration: use `paths.builds` as the single configurable root for all disposable/generated data, with Ludared managing internal directories such as `cache/decodes` itself. Keep separate paths only if a concrete use case requires them (shared cache, separate storage, CI, etc.).

### Other stuff
- [ ] Migrate to [usage-rs](https://usage.jdx.dev/rust/migrating-from-clap#migrating-from-clap)
- [ ] Interace with [ratatui](https://ratatui.rs/installation/)

## Notes

This document is a lightweight roadmap while the CLI evolves.

```json
{
  "decodes": {
    "my_rom.sfc": [
      {
        "name": "rom_banks",
        "codec": {
          "id": "std/nintendo/snes/cart/lorom",
          "version": 1,
          "args": {
            "bank_numbers": "mapped"
          }
        },
        "outputs": [
          "rom_bank_80.bin", "etc..."
        ],
        "decodes": {
          "rom_bank_80.bin": [
            {
              "name": "rom_name.txt",
              "codec": {
                "id": "std/extract_bytes",
                "version": 1,
                "args": {
                  "target": "rom_name.txt",
                  "offset": 0,
                  "length": 21
                }
              },
              "outputs": [
                "ROM_NAME.txt"
              ]
            }
          ]
        }
      }
    ]
  }
}
```
