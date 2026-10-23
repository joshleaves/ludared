---
name: ludared-lua
description: Implement, modify, review, debug, or test Lua codecs for Ludared projects. Use for Lua codec behavior, registration, decode/encode contracts, round-trip failures, and script-specific errors; not merely because a project contains Lua files.
---

# Developing Ludared Lua codecs

Use this skill for **codec development**, not ordinary work on a Ludared project. For the complete Lua module API, globals, signatures, and error messages, consult [`references/lua-api.md`](references/lua-api.md) before implementing or changing a codec. Check the current implementation when the reference and runtime disagree.

You can also use `ludared codecs create [my-codec-name].lua` to create a skeleton codec in "{paths.scripts}/[my-codec-name].lua". Intermediate directories are created automatically. Paths are relative to paths.scripts. Components `..`, `shared`, and `libraries` are forbidden anywhere in the path.

## Mental model and contracts

- A decode applies **one codec to one input** and records **one decode node with an ordered list of zero or more output artifacts**. A codec does not create other decode nodes or edit the manifest. Additional decode steps are declared separately through Ludared.
- Outputs are returned as an **array of `{ name = ..., data = ... }` records**, not a map keyed by filename. The order is significant and is preserved for `encode`.
- Output names are relative artifact paths: no leading `/`, `..`, duplicates, or ancestor/descendant conflicts between outputs of the same decode.
- Lua codecs may read project-specific resources through `LUDARED.WORKSPACE`, including lookup tables, databases, and files from other branches. These reads can be untracked dependencies; document them. During `decode`, prefer stable, pristine reference data rather than workspace overrides whose presence or content can change.
- Treat the workspace and project files as read-only during Ludared operations (a contract, not an enforced sandbox). Codecs must return generated artifacts rather than writing them to disk, and must not rely on mutations by other codecs. Independent branches have no guaranteed execution order; successive patches on the same node follow their declared order.
- `kind = "rebuild"`: `encode` reconstructs the complete parent from its ordered outputs and receives an empty `original`. Think of rebuilding a ZIP from its extracted files.
- `kind = "patch"`: `encode` receives the current parent bytes and returns them with targeted modifications, like applying an IPS-style patch. Successive patches on one node follow their declared order; do not assume an order between independent branches.
- Unmodified artifacts should round-trip to the original bytes. Do not rely on persistent Lua state or execution timing. Validate the count, names, order, sizes, and consistency of artifacts received by `encode`: overrides may have changed them. Project-specific read-only resources are permitted; avoid volatile external inputs when reproducibility matters. Arguments are an opaque string (or `nil`) recorded by Ludared and passed to both operations.

## Confidence is evidence, not certainty of uniqueness

`can_handle` returns the Lua equivalents of `CodecHandlingConfidence`:

- `"no"`: "I cannot decode that data meaningfully, no way."
- `"possible"`: "I may extract something out of that data, maybe garbage, maybe gold."
- `"likely"`: "All signs point to yes, but false positives are possible."
- `"certain"`: "I know that data, I know that format, I'm in charge."

`likely` does not require proof that no other format could match. Use `possible` when evidence that extraction will be meaningful is insufficient. `ludared decode add` refuses `no` unless `--force` is supplied and warns on `possible`. Choose confidence based on evidence, not CLI warnings. Detection must not mutate project state or produce artifacts.

## Project layout and discovery

- Codecs are Lua files under `paths.scripts` (normally `scripts/`), identified by their path relative to that root, **including `.lua`**. The module's `name` is a display name, not its ID.
- Every `*.lua` file below the scripts root is discoverable as a codec except files beneath directories the runtime explicitly excludes (currently `shared/` and `libraries/`). Put helpers there. `tests/` is a possible future convention, **not** a confirmed exclusion; do not put discoverable test scripts there without checking the implementation.
- Discovery eagerly loads codec modules. **One malformed codec can break all project commands**. Keep module initialization free of side effects.
- Each codec invocation uses a fresh Lua VM. Do not rely on state persisting between `decode`, `encode`, or detection.
- Ludared embeds Lua **5.5** through **mlua**. Use Lua 5.3+ facilities such as `string.pack`, `string.unpack`, and integer bitwise operators for binary parsing. Verify availability of third-party modules in the actual runtime.
- `require` does not automatically search the scripts root. Consult the API reference for `dofile(LUDARED.SCRIPTS .. "/shared/...")` or explicit `package.path` setup.
- Do not assume a bundled JSON or SQLite module; verify available dependencies. Codec arguments are opaque to Ludared, and metadata representation is codec-specific.

## Implementation workflow

1. **Inspect first.** Check `ludared codecs list`, `ludared codecs info <codec-id>`, nearby project codecs, and the Lua API reference. Prefer existing Ludared operations and cached pristine artifacts over implementing a parallel extractor.
2. **Define the contract.** Decide `patch` versus `rebuild`; specify input recognition, output names and order, required reconstruction information, argument syntax, and round-trip expectations.
3. **Implement narrowly.** Return only declared artifacts; validate offsets, lengths, arguments, and structural assumptions with useful errors. Do not introduce unrelated project or manifest changes.
4. **Preserve reconstruction information.** If rebuilding needs data not captured by the main extracted files (such as layout or ordering), emit it as an ordinary artifact. A metadata sidecar is **one strategy**, not a required format, position, or module structure. It can be overridden like other terminal artifacts.
5. **Validate on representative inputs**, including malformed cases and boundary conditions. Test both unchanged round-trips and intentional edits. Use local real-world samples when available and permitted, but do not commit or redistribute commercial game data as fixtures.

## CLI validation checklist

Run commands from the project directory (project lookup and configured paths are CWD-relative):

```sh
ludared codecs list
ludared codecs info scripts/my_codec.lua
ludared unpack
ludared decode add <input-virtual-path> scripts/my_codec.lua
ludared artifacts list --tree
ludared cache cat <output-virtual-path>
ludared build <input-virtual-path>
```

Use the actual virtual paths and codec ID for the project. `decode add` may require the input to be populated in the cache first; run `unpack` before it. Compare an untouched build byte-for-byte with its original source where appropriate. Then override a terminal artifact and verify that the change appears in the rebuilt parent without altering unrelated bytes. If a codec fails to load, check the most recently added Lua module before debugging the entire project.

## API details

Read [`references/lua-api.md`](references/lua-api.md) for the required module fields, exact Lua signatures, `LUDARED` globals, module loading examples, and error catalogue. Treat examples there as API demonstrations, not mandatory implementation patterns.

