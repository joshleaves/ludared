# ludared

ludared (LUDus ARchive EDitor) is a command-line tool to help build and maintain ROM hacking and game modding projects.

## Shell completions

ludared provides dynamic shell completions, including project-aware completion for sources and other project resources.

Use `ludared completions [SHELL] | source`. If you don't provide a shell, it will be identified best-effort from your `$SHELL`environment variable.

# Current status
> **Status:** Early development.


## Available commands

- [x] `doctor` — Verify project configuration and source files.
- [x] `clean` — Remove generated build and cache artifacts.
- [x] `sources list` — List configured source files.
- [x] `sources add` — Add a source file to the manifest.
- [x] `sources remove` — Remove a source file from the manifest.

## Planned commands

- [ ] `init` — Set up a project
- [ ] `unpack`
- [ ] `repack`
- [ ] `build`
- [ ] `extract`
- [ ] `edit`
- [ ] `archive`
- [ ] `tool`

## Notes

This document is a lightweight roadmap while the CLI evolves.