# ludared

ludared (LUDus ARchive EDitor) is a command-line tool to help build and maintain ROM hacking and game modding projects.

## Shell completions

ludared provides dynamic shell completions, including project-aware completion for sources and other project resources.

Use `ludared completions [SHELL] | source`. If you don't provide a shell, it will be identified best-effort from your `$SHELL` environment variable.

# Current status
> **Status:** Early development.

## Available commands

- [x] `init` - Set up a project

- [x] `sources list` - List configured source files
- [x] `sources add` - Add a source file to the manifest
- [x] `sources remove` - Remove a source file from the manifest

- [x] `codecs list` - List available codecs
- [x] `codecs info` - Get information about a specific codec
- [x] `codecs detect` - Detect which codecs can be used on a file

- [x] `decode add`  - Add a decode step

- [x] `doctor` - Verify project configuration and source files
- [x] `clean` - Remove generated build and cache artifacts

- [x] `completions` - Generate completions for your shell

## Available completions
- [x] `codecs info => complete_codecs_list` - Available codecs
- [x] `sources add => complete_source_add` - Available files in `sources/` folder, and not in manifest
- [x] `sources remove => complete_source_remove` - Sources declared in manifest

## Planned commands

- [ ] `decode`
  - [x] `add` - `decode add <VPATH> <CODEC> [ARGS] [NAME]`
  - [ ] `list` - `decode list [VPATH]`
  - [ ] `remove` - `decode remove <VPATH> <NAME> `

- [ ] `cache`
  - [ ] `ls` - `cache list [VPATH*]`
  - [ ] `cat` - `cache cat <VPATH>`
  - [ ] `path` - `cache path <VPATH>`


- [ ] `unpack`
- [ ] `build`

- [ ] `archive`
- [ ] `tool`

## To-do list

### Codecs
- [ ] Add versioning and metadata, with an ABI-friendly key/value metadata representation for future dynamic plugins.

### Configuration
- [ ] Reconsider build/cache path configuration: use `paths.builds` as the single configurable root for all disposable/generated data, with Ludared managing internal directories such as `cache/decodes` itself. Keep separate paths only if a concrete use case requires them (shared cache, separate storage, CI, etc.).

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