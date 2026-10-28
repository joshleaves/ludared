# Scope of `codecs/common`

This directory contains small, format-agnostic implementation shared by codecs and codec tests. Keep code here only when it expresses a reusable operation intrinsic to decoding or encoding binary formats and multiple codecs need it (or there is a clearly approved codec-facing reuse such as LuaCodec hashing). Prefer descriptive domain modules when they explain a shared responsibility better than `common`; keep format-specific concepts, layout rules, metadata conversions and one-off utilities in the relevant codec/domain module.

## Modules

- `bytes.rs`: bounds-checked reads of fixed byte fields and endian integer values from codec input buffers. Keep the `read_` names and return errors for out-of-bounds fields; do not add unchecked reads or general buffer-management APIs.
- `hash.rs`: hashing operations needed by codecs for format-defined digests. `sha256` returns raw digest bytes for formats that store binary SHA-256 values.

Do not place codec-private utilities here, and do not grow this into a general-purpose utility bucket.

## Do not reuse for source verification

`sha256` here is for codec data (for example, hashes embedded in a file format). It is not the project-source integrity/checksum API. Do not replace `Source::compute_sha256` or `Source::sha256_matches` with this helper: those operations belong to source verification, and `compute_sha256` intentionally hashes files incrementally without loading the whole file into memory. If another non-codec subsystem appears to duplicate hashing, preserve its own semantics and ownership unless a separate design decision explicitly moves that responsibility.

LuaCodec's `LUDARED.sha256` is an allowed exception: it is a codec-facing scripting API, so it may call this codec helper as long as its output remains the same lowercase hexadecimal SHA-256 string and its existing tests pass. Convert the raw digest with `hex::encode`; do not change the Lua API or source-verification implementation.

When changing these modules, keep unit tests for endian correctness, truncation behavior, and known hash vectors. Run their tests and the full relevant codec test suite.
