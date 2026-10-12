# Codec Contract

This document defines the contract between ludared's codec implementations,
manifest, and decode/rebuild pipeline. It describes intended guarantees, not
implementation details of individual formats.

**Status:** architectural contract. Requirements marked **TO VERIFY** must be
checked against the pipeline before being treated as implemented guarantees.

## 1. Identity and configuration

- Each codec has a stable, unique `id` and exposes its `kind`.
- A decode node records the codec ID, version, and arguments needed to replay
  the operation. The framework must not infer codec behavior from its ID string.
- The codec owns the meaning and validation of its arguments. The same arguments
  apply to its matching `decode` and `encode` operations.
- Codec versions and migration policy are not yet defined by this contract.

## 2. Codec kinds and composition

There are two codec kinds:

- **Rebuild:** constructs the complete parent artifact from its outputs.
  `encode` receives an empty input buffer (`Vec::new()`).
- **Patch:** modifies an existing parent artifact. `encode` receives ownership
  of the current parent buffer and returns the modified buffer.

For a given parent artifact, the manifest permits **one Rebuild or multiple
Patches**, never a mixture. Multiple Patches are applied in manifest declaration
order. A later Patch may overwrite bytes written by an earlier Patch.

`encode` deliberately accepts an owned `Vec<u8>`: a Patch can modify and return
that allocation without cloning the entire parent buffer.

## 3. Decode outputs

- `decode` returns an **ordered sequence** of `DecodedArtifact` values.
- Each output has a name and bytes. Output names are relative artifact paths,
  not absolute paths; they must satisfy `VirtualPath`'s path rules.
- Names must be unique within the decode, and outputs attached to the same
  parent must not duplicate or overlap existing output paths. An output cannot
  be both an artifact and the implicit parent directory of another output.
- The codec chooses the output order. **Canonical** means deterministic for the
  same input and arguments; it does **not** mean alphabetical or numerically
  sorted.
- The manifest records this order in `DecodeNode.outputs`. Neither the manifest
  nor the pipeline may sort it for presentation convenience and then use that
  reordered sequence for encoding.

## 4. Encode inputs and ordering

- `encode` receives outputs in **exactly the order recorded in
  `DecodeNode.outputs`**. **TO VERIFY:** the rebuild pipeline must enforce this
  before the rule can be considered an implemented guarantee.
- An output name identifies an artifact; the framework assigns it **no implicit
  physical offset**. A codec may interpret names when its format requires it,
  but that is a codec-specific rule, not a framework-wide convention.
- Codecs must not assume their inputs have been alphabetically sorted.
- Reordering `DecodeNode.outputs` is a meaningful manifest edit: it may change
  the reconstructed bytes. The framework must not silently repair the order.
- The codec is responsible for validating the completeness, sizes, and other
  format-specific constraints of the artifacts it receives. It must fail rather
  than invent missing content or silently shift data to conceal a gap.

For example, a bank codec may emit `rom_bank_c0.bin`, `rom_bank_c1.bin`, and
`rom_bank_c2.bin` in physical order. The names do not, by themselves, require
the framework to place the banks at offsets derived from `c0`–`c2`. If a bank is
missing, the codec must apply an explicit format-specific policy; it must not
silently fill a hole with arbitrary bytes.

## 5. Round-trip behavior

For a **lossless, reversible** codec, an unchanged decode/encode cycle must
reproduce the original bytes exactly, including headers and otherwise
uninterpreted data:

- Rebuild: `encode(Vec::new(), decode(input, args), args) == input`.
- Patch: `encode(input.to_vec(), decode(input, args), args) == input`.

These are conceptual identities: the actual calls pass the ordered artifact
sequence. The same `args` MUST be used for decoding and encoding. A codec
that cannot promise byte-exact reversibility must document that limitation
explicitly; the framework must not silently assume it is reversible.

Round-trip tests should cover relevant argument modes and edge cases, not only
the default configuration.

## 6. Errors and boundaries

- Invalid output names, duplicate names, and overlapping outputs are errors,
  not conditions to normalize silently.
- Distinguish codec-produced invalid outputs from collisions with outputs
  already declared in the manifest.
- Invalid arguments, missing required artifacts, incompatible sizes, and
  impossible reconstructions must return errors with useful context.
- The framework handles manifest registration, cache/workspace lookup, and
  orchestration. Codecs handle format interpretation and byte transformations.
- A codec must not mutate the manifest or decide how artifacts are stored.

## 7. Verification checklist

Before treating the ordering contract as implemented, verify that:

1. `decode` outputs are registered in their original order.
2. The rebuild pipeline resolves and supplies artifacts in
   `DecodeNode.outputs` order, including overridden artifacts.
3. Multiple Patches execute in declaration order and reuse the current parent
   buffer; Rebuild and Patch cannot coexist on one parent.
4. Relevant codecs reject missing or incompatible inputs rather than silently
   generating filler or shifting positions.
5. Tests cover byte-exact round trips and, where meaningful, deliberate
   changes to output ordering.
