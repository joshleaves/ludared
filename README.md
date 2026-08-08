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
- [x] `codecs info` - Available codecs
- [x] `sources add` - Available files in `sources/` folder, and not in manifest
- [x] `sources remove` - Files in manifest

## Planned commands

- [ ] `unpack`
- [ ] `repack`
- [ ] `build`
- [ ] `extract`
- [ ] `edit`
- [ ] `archive`
- [ ] `tool`

## To-do list

### Codecs
- [ ] Add versioning and metadata, with an ABI-friendly key/value metadata representation for future dynamic plugins.

## Notes

This document is a lightweight roadmap while the CLI evolves.