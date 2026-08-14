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

- [x] `doctor` - Verify project configuration and source files
- [x] `clean` - Remove generated build and cache artifacts

- [x] `completions` - Generate completions for your shell

## Available completions
- [x] `codecs info => complete_codecs_list` - Available codecs
- [x] `sources add => complete_source_add` - Available files in `sources/` folder, and not in manifest
- [x] `sources remove => complete_source_remove` - Sources declared in manifest

## Planned commands

- [ ] `decode`
  - [ ] `list` - `list [VPATH]`
  - [ ] `add` - `add <VPATH> <CODEC> [NAME] [ARGS]`
  - [ ] `remove` - `remove <VPATH> <NAME> `

- [ ] `unpack`
- [ ] `build`

- [ ] `archive`
- [ ] `tool`

## To-do list

### Codecs
- [ ] Add versioning and metadata, with an ABI-friendly key/value metadata representation for future dynamic plugins.

## Notes

This document is a lightweight roadmap while the CLI evolves.


```json
{
  "unpacks": [
    {
      "input": "DBZ.sfc",
      "codec": {
        "id": "std/nintendo/snes/cart/lorom",
        "args": {}
      },
      "outputs": [
        "rom_00.bin",
        "rom_01.bin"
      ],
      "unpacks": [
        {
          "input": "rom_00.bin",
          "codec": {
            "id": "srd/extract_texture",
            "args": {}
          },
          "outputs": [
            "screen_title.bmp",
            "screen_title.colors.tex"
          ]
        }
      ]
    }
  ]
}
```