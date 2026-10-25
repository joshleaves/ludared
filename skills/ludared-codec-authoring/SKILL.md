---
name: ludared-codec-authoring
description: Implement, review or modify a native Rust codec inside the Ludared repository. Covers the Codec trait contract, Rebuild and Patch kinds, artifact naming and ordering, codec arguments, metadata sidecars, registration, error handling, and the unit, end-to-end and fixture conventions the repository actually follows. Use when adding or changing a file under src/codecs/, registering a codec, or writing codec tests in this repository.
---

# Authoring native codecs for Ludared

A codec turns a blob of bytes into named artifacts, and puts them back
together. The framework owns paths, caching, overrides and orchestration; the
codec owns format interpretation and byte transformations.

## Scope: change the codec, not the framework

**An agent implementing one codec must not modify the CLI, the manifest
format, the `Codec` trait, the registry's structure, or unrelated codecs.**

Those are shared abstractions. Changing them to make one codec easier is how
unrelated behaviour breaks. The only files a new codec normally touches:

```
src/codecs/<family>/<format>.rs           the codec (or <format>/mod.rs)
src/codecs/<family>/mod.rs                 one `pub(crate) mod <format>;` line
src/codecs/mod.rs                         one `pub(crate) mod <family>;` line
src/codecs/registry.rs                    one entry in `BUILTINS`
src/testing/codecs/e2e/<family>/<codec>.rs (optional) end-to-end properties
src/testing/codecs/e2e/<family>/mod.rs      one `pub(crate) mod <codec>;` line
src/testing/fixtures/...                  (optional) a binary fixture
```

If a codec genuinely cannot be written without a change to a shared
abstraction, say so and explain why before writing it. A codec that needs a
new hook in the trait, a new CLI flag, or a change to the manifest format is a
sign that the design is being forced.

## The trait

`src/codecs/mod.rs` defines:

```rust
pub trait Codec: Send + Sync {
  fn id(&self) -> &str;
  fn name(&self) -> &str;
  fn description(&self) -> &str;
  fn kind(&self) -> CodecKind;
  fn can_handle(&self, data: &[u8]) -> Result<CodecHandlingConfidence, CodecError>;
  fn decode_name(&self, args: Option<&str>) -> Result<String, CodecError>;
  fn decode(&self, data: &[u8], args: Option<&str>) -> Result<Vec<DecodedArtifact>, CodecError>;
  fn encode(&self, original: Vec<u8>, artifacts: &[DecodedArtifact], args: Option<&str>)
    -> Result<Vec<u8>, CodecError>;
}

pub struct DecodedArtifact {
  pub name: String,
  pub data: Vec<u8>,
}
```

Implementations in this repository are unit structs (`pub(crate) struct Extract;`)
implementing `Codec`, with the metadata in module-level constants:

```rust
const CODEC_ID: &str = "std/generic/extract";
const CODEC_NAME: &str = "Extract bytes";
const CODEC_DESC: &str = r#"Raw bytes extractor
...
"#;
```

The description is user-facing: it documents the arguments, the guarantees and
the limits. Follow the existing ones in tone and detail.

### `kind`

```rust
pub enum CodecKind { Rebuild, Patch }
```

- `Rebuild` constructs the whole parent from its artifacts. `encode` receives
  an **empty** `original` and must not read it.
- `Patch` modifies the current parent. `encode` receives the parent's bytes by
  value and returns the modified buffer, preserving everything the artifacts do
  not cover.

Think of Rebuild as reconstructing a ZIP archive from its extracted files, and
Patch as applying modifications to an existing stream, similarly to an IPS patch.

The framework enforces that an artifact carries **one Rebuild or several
Patches, never a mix**. Several patches are applied in manifest order, each
over the buffer the previous one returned, so a later patch may overwrite an
earlier one.

### `can_handle`

```rust
pub enum CodecHandlingConfidence { No, Possible, Likely, Certain }
```

- `No`: "I cannot decode that data meaningfully, no way."
- `Possible`: "I may extract something out of that data, maybe garbage, maybe gold."
- `Likely`: "All signs point to yes, but false positives are possible."
- `Certain`: "I know that data, I know that format, I'm in charge."

`Likely` does not require proof that no other format could match. It means
the available evidence strongly suggests that the codec can meaningfully
decode the input. The mere possibility of false positives is not a reason
to downgrade to `Possible`.

Reserve `Possible` for cases where the codec can attempt extraction but
has insufficient evidence that the result will be meaningful.

`ludared decode add` refuses `No` unless `--force` is given, and warns on
`Possible`. These warnings reflect the confidence contract; do not
artificially lower or raise confidence to influence CLI behavior.

Detection must not have side effects or produce artifacts.

### `decode_name`

Returns the name of the decode node. It must not depend on the input bytes.
It may depend on `args`. `ludared decode add --name` overrides it.

## Artifact naming and ordering

`decode` returns an **ordered** `Vec<DecodedArtifact>`. The framework:

- records the names in the manifest in that order, and never sorts them;
- hands them back to `encode` in exactly that order;
- refuses duplicate names within one decode;
- refuses two outputs of one decode standing in an ancestor relation
  (`data` and `data/head.bin`);
- refuses an output overlapping one another decode on the same input already
  declared.

Names are relative artifact paths: no leading `/`, no trailing `/`, no `//`,
no `.` or `..` component, no backslash. The framework validates them; the
codec must produce valid ones.

**Order is meaningful.** If the format has a physical order (banks in an image,
files on a disc), emit the artifacts in that order and let `encode` rely on
it. Do not sort by name: `rom_bank_10` sorts before `rom_bank_2`, and the
framework assigns no physical offset to a name.

## Arguments

Arguments are **opaque strings**. The framework parses nothing, validates
nothing, and records them verbatim, so the bytes `decode` sees are the bytes
`encode` sees.

A codec that wants typed arguments derives them from the string itself, using
the helper the repository provides:

```rust
#[derive(Debug, Deserialize)]
struct RawSplitCodecArgs {
  offsets: Vec<usize>,
  pattern: Option<String>,
}

impl LudaredCodecArgs for RawSplitCodecArgs {}

let args = RawSplitCodecArgs::from_args(args)?;   // {} when args is None
```

`LudaredCodecArgs::from_args` parses `args.unwrap_or("{}")` as JSON, so a
codec with no arguments still gets a valid empty object. Use
`#[serde(default)]` for optional fields, and `#[serde(deny_unknown_fields)]`
where a typo should be an error rather than a silent default.

The same argument struct is used by `decode` and `encode`.

## Metadata sidecars

Some formats need reconstruction information that is not represented by the
other decoded artifacts (for example, file order or a partition table).

One possible strategy is to emit a metadata artifact alongside the extracted
files. The ISO9660 and CCI codecs place this artifact first, allowing `encode`
to retrieve it by position:

```rust
let mut artifacts = Vec::with_capacity(entries.len() + 1);
artifacts.push(DecodedArtifact { name: METADATA_NAME.to_owned(), data: metadata.to_json()? });
artifacts.extend(entries.into_iter().map(|entry| DecodedArtifact { ... }));
````

On the encode side:
```rust
let (metadata_artifact, files) = artifacts.split_first().ok_or_else(|| {
  CodecError::Message(format!("Expected a metadata artifact, found none among {} artifacts", artifacts.len()))
})?;
```
This is an implementation strategy, not a framework requirement.

The metadata is a regular artifact: it is listed, can be overridden, and
modifying it affects reconstruction. The codec must validate consistency
between its metadata and the other artifacts.

For substantial metadata handling, a separate module may be useful.
Existing examples:

* src/codecs/cdrom/iso9660/metadata.rs
* src/codecs/nintendo/n3ds/container/cci/metadata.rs

## Round-trip expectations

For a lossless codec:

```
Rebuild: encode(Vec::new(), decode(input, args), args) == input
Patch:   encode(input.to_vec(), decode(input, args), args) == input
```

Bit-exactness is a **claim a codec makes and its tests prove**, not a default.
The README's table marks each codec BE (bit-exact) and Det. (deterministic); a
codec must pass the corresponding end-to-end test before it may claim either.
`N/A` means "not yet verified", not "does not hold".

Legitimate exceptions, which must be documented in the codec's description:

- the format itself does not round-trip (the ISO 9660 codec writes plain
  Level 1 images, so Rock Ridge data is lost);
- a checksum or a header field is recomputed (the SNES cartridge codecs
  finalise the checksum, so an image that never had one comes back with one);
- the format forbids a change the codec refuses to make (the CCI codec refuses
  to grow a partition past the declared image size).

A codec that cannot promise a byte-exact round trip says so in its
description. It must never silently produce something else.

## Errors

Return `CodecError`. Use `CodecError::Message(format!(...))` for anything the
format itself refuses, and put the offending values in the message:

```rust
return Err(CodecError::Message(format!(
  "Too many ROM banks: {} (Max for LoROM: 128)",
  bank_count
)));
```

Rules:

- **Fail rather than guess.** A missing artifact, an incompatible size or an
  impossible layout is an error, never a zero-filled hole or a shifted offset.
- Distinguish "the codec's own input is wrong" from "the framework gave me
  the wrong artifacts" in the message.
- Validate artifacts in `encode`: the right count, the right names, the right
  sizes. The framework does not check them for you.
- Never mutate the manifest, the cache or the workspace from a codec.

## Registration

A built-in codec is registered in `src/codecs/registry.rs`:

```rust
static BUILTINS: &[&dyn Codec] = &[
  &N3dsCci,
  &Extract,
  &Iso9660,
  ...
];
```

The id in the registry is the id in `CODEC_ID`; `ludared codecs list` shows
them sorted by id. Ids follow `std/<family>/<format>`, with `/` as the
separator (the CLI renders them as a tree).

Registration is the only framework change a new codec needs.

## Tests

Three layers, each with a different job. Use all three that apply.

`ludared scripts test [PATH*]` runs project Lua test scripts, not these Rust
tests. It can exercise builtins through `LUDARED.codec(id)`; see
`ludared-lua` for script layout and execution. Keep the native codec's Rust
unit and end-to-end coverage below.

### Unit tests — in the codec's own file

A `#[cfg(test)] mod tests` at the bottom of the codec file. They cover the
codec in isolation: argument parsing, the refusal cases, the ordering.

```rust
#[test]
fn refuses_an_extraction_whose_end_overflows() {
  let args = args("overflow.bin", usize::MAX - 2, 8);
  let Err(err) = Extract.decode(SOURCE, Some(&args)) else {
    panic!("an overflowing window should be refused");
  };
  assert!(err.to_string().contains("overflows"), "{err}");
}
```

Shared test helpers live beside the codecs they serve, not in the codec file:
`src/codecs/nintendo/snes/common/testkit.rs` builds SNES images and asserts
round trips and header finalisation for every cartridge codec.

### End-to-end tests — `src/testing/codecs/e2e/`

Three properties, implemented by the `CodecE2E` trait in
`src/testing/codecs/e2e/mod.rs` and wired up by the `codec_e2e_tests!` macro:

| Property | Asserts |
|---|---|
| `bit_exact` | `encode(&[], &decode(input), args) == input` (the input is passed as `original` for a patch codec) |
| `decode_golden` | the decode produces the recorded artifacts, identified by name, size and SHA-256 |
| `encode_golden` | `encode(artifacts, args)` produces the recorded bytes, pinned to a SHA-256 |

```rust
pub(crate) struct N3dsCciE2E;

impl CodecE2E for N3dsCciE2E {
  const CODEC: &'static dyn Codec = &N3dsCci;

  fn bit_exact_fixtures() -> impl Iterator<Item = BitExactFixture> {
    std::iter::once(BitExactFixture { name: "nintendo_3ds_cci.3ds", input: SOURCE.to_vec(), args: None })
  }
  // ...
}

codec_e2e_tests!(N3dsCciE2E, bit_exact, decode_golden, encode_golden);
```

Registering a property without a fixture fails the test, so a codec cannot
claim a guarantee it has not demonstrated. Only register the properties the
codec actually satisfies.

### CLI-level tests — `tests/`

`tests/cli_build.rs`, `tests/cli_codecs.rs` and `tests/cli_override.rs` drive
the real binary against a throwaway project. Use them when the codec's
behaviour depends on the framework (ordering across the manifest, overrides,
the cache). They are the right place for "a decode through the CLI produces
these artifacts and rebuilds the source".

## Fixtures and their licensing

Binary fixtures live under `src/testing/fixtures/<family>/`, next to a
`README.md` that records:

- what the fixture is and how it was made;
- its SHA-256, and the SHA-256 of the rebuilt image;
- the licence of the fixture **and of anything embedded in it**.

Rules the repository follows:

- A fixture that is hand-made or synthetic is dedicated to the public domain
  under **CC0 1.0**.
- A fixture that embeds third-party material says so, and that material keeps
  its own terms. The SNES fixture embeds a Phrack article and says so.
- Fixtures are generated by a committed script when possible
  (`src/testing/fixtures/n3ds/nintendo_3ds_cci.py`), and the expected hashes
  are pinned only after an independent tool agreed with them.

Two things to check when you add a fixture:

1. The `exclude` glob in `Cargo.toml` is supposed to keep binary fixtures out
   of the published package. **Check that the glob matches where you put the
   file** — the current one (`src/testing/fixtures/data/**`) does not match the
   directories the fixtures actually live in, so it is not doing its job.
2. The fixture is read with `include_bytes!` from a path relative to the file
   that uses it.

Do not add a fixture copied from a game or a commercial release. If a test
needs real data, generate synthetic data instead.

Never commit commercial game data or copyrighted assets as test fixtures.Use
synthetic or appropriately licensed fixtures for repository tests.

When available and legally permitted, locally accessible commercial game data
may be used for additional validation and edge-case testing. Such data must
remain outside the repository’s tracked fixtures and must never be committed
or redistributed.

Do not copy commercial data into generated fixtures or test snapshots.

## Repository conventions

- **Formatting**: `cargo fmt --all` (2-space indent, `rustfmt.toml`).
- **Lints**: `cargo clippy --all-targets --all-features -- -D warnings` must be
  clean.
- **Tests**: `cargo test --all-features` must pass. CI runs all three.
- **Toolchain**: Rust 1.99.0 (`rust-toolchain.toml`), edition 2024.
- Comments explain *why*, not *what*. The existing code is heavily commented
  with the reasoning behind each decision; match that.
- Error messages state what was expected and what was found, with the
  offending values in them.
- `let ... else` is the idiom used for validation guards in the newer code.

## Check list

- [ ] The codec compiles, `cargo fmt --all -- --check` is clean, clippy is
      clean.
- [ ] `can_handle` says `No` for foreign data and `Certain` only when sure.
- [ ] `decode` emits valid, unique, non-overlapping names in a deliberate
      order.
- [ ] `encode` validates the artifacts it is handed and refuses anything it
      cannot reconstruct.
- [ ] The round trip is proven by a test, or the exception is documented in
      the description.
- [ ] Arguments are parsed by the codec, recorded verbatim, and used
      identically on both sides.
- [ ] The codec is registered in `BUILTINS`.
- [ ] Unit tests cover the refusals; end-to-end properties cover the
      guarantees the codec claims.
- [ ] Any new fixture has a README with its provenance and licence.
- [ ] No file outside the codec and its tests was modified.
