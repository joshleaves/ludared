# Nintendo DARC/SARC and LZ10/LZ11 fixtures

These six files are produced by **third-party implementations**, never by
Ludared, so the end-to-end tests that read them check interoperability rather
than self-consistency. All content is synthetic and dedicated to the public
domain under **CC0 1.0**; the fixtures embed no game data and no third-party
code.

| Fixture | Bytes | SHA-256 |
|---|---:|---|
| `nintendo_darc.arc` | 2324 | `e71e88d56e742ef34f0e686445fd915d3c8166eec5a172a437d335375147b0b9` |
| `nintendo_sarc.sarc` | 2560 | `4d069e0c8bdac130d2706d8e9115cb5ba160b9c3296adc63aa638b89db19fd34` |
| `nintendo_darc.lz10` | 1208 | `38f391e018c02efc4733e362cc1dfa815d565032d4dc43680a43fa5d728fa407` |
| `nintendo_darc.lz11` | 1060 | `2e829cd559fbbe0fe358e46a1e88bfc5efccc833908faa5b2fb411bfac8c3e22` |
| `nintendo_sarc.lz10` | 1224 | `2d6cbe40e462a30873c7c8a4bc4d0eff1bb2b5b8c3c4d07dd92eb345dc3bd944` |
| `nintendo_sarc.lz11` | 1044 | `03d7fe6ad2fa691a03c3f40ef12a12ebc1880a7e12f02ae3718412e45312133b` |

## Tools

| Format | Implementation | Revision | License |
|---|---|---|---|
| DARC | [dnasdw/darctool](https://github.com/dnasdw/darctool) 1.2.0 | `004ca96` (2022-10-02) | no license file in the repository; used locally, not redistributed |
| SARC | [Tyulis/3DSkit](https://github.com/Tyulis/3DSkit) | `2fb6077` (2024-08-25) | GPL-3.0 |
| LZ10/LZ11 | [magical/nlzss](https://github.com/magical/nlzss) (`compress.py`, `lzss3.py`) | `f27414f` (2014-05-14) | MIT |

None of them is a build or test dependency: the Rust tests read the committed
bytes and never shell out. The generator below is the only code that talks to
the tools, and it is run by hand. Local checkouts belong under the ignored
`DO_NOT_COMMIT/TOOLS/` directory; they are not part of the project or its
releases.

## Content

Every archive carries the same three files, built by
`generate_nintendo_archives.py` from a fixed seed (`0x4C554441`, xorshift32):

| Path | Bytes | SHA-256 |
|---|---:|---|
| `hi.txt` | 20 | `2b60e2f2eb8a463e852baef8c59d817fccbe59f3ebcf029bee382802686a3e64` |
| `data/pattern.bin` | 1280 | `892d09da5f31b759ad025a291ce85055e3e34ada6b19d726ef41a40b9ca4f3f7` |
| `data/nested/noise.bin` | 777 | `ca8f7586dfc69e7c8353dda6160ff2faa6d65a5684cf19a2836a32262fc383d1` |

`hi.txt` is plain text, `data/pattern.bin` repeats an eight-byte pattern 160
times (highly compressible) and `data/nested/noise.bin` is a pseudorandom
stream (incompressible). The tree has two directory levels, and the sizes
differ so layout and reference encodings both vary.

### DARC layout

`dnasdw/darctool -cvfd … -a 32` writes a little-endian archive with the `.`
root entry, 32-byte data alignment and its own file order:

```
0x0000  darc header, version 0x01000000, header 0x1c
0x001c  entry table, 7 entries (84 bytes)
0x0070  UTF-16 name area (0x70 bytes)
0x00e0  data area

[0] ""            root    parent 0  end 7
[1] "."           folder  parent 0  end 7
[2] hi.txt        file    offset 0x900  size 0x014
[3] data          folder  parent 1  end 7
[4] pattern.bin   file    offset 0x400  size 0x500
[5] nested        folder  parent 3  end 7
[6] noise.bin     file    offset 0x0e0  size 0x309
```

The parent indices follow Nintendo's own convention (verified against retail
archives): the children of `.` point at index 1, and `nested` points at `data`
(index 3).

### SARC layout

`3DSkit -p -f SARC -l -d` writes a little-endian SARC with a 0x80-byte data
alignment, hash-sorted SFAT and the file names in SFNT:

```
0x0000  SARC header, version 0x0100, data offset 0x100
0x0014  SFAT, 3 nodes, hash multiplier 0x65
0x0054  SFNT with the three names
0x0100  data area (pattern.bin, then hi.txt 0x80-aligned, then noise.bin)
```

Node order is the name-hash order 3DSkit computes (`data/pattern.bin`,
`hi.txt`, `data/nested/noise.bin`), which is why the decoded artifact order
differs from the DARC one. The declared file size is `0x989`; the file is
padded to `0xa00` after the last node, which the codec keeps as trailing bytes.

### LZ streams

`nintendo_darc.lz10`/`.lz11` and `nintendo_sarc.lz10`/`.lz11` are nlzss's
`compress()` and `compress_nlz11()` output over the two archives above. Their
headers are the plain LZ10/LZ11 size fields (`10 09 09 00`, `11 09 09 00`,
`10 00 0a 00`, `11 00 0a 00`), so the archives are stored uncompressed inside
the stream, not wrapped in a container signature.

## Independent validation

Performed once when the fixtures were generated, and available again through
`generate_nintendo_archives.py --check`:

- `darctool -xvfd` re-extracts the DARC fixture to the three files above,
  byte-identical to the source content.
- `3DSkit -x -f SARC` re-extracts the SARC fixture to the same three files,
  byte-identical to the source content.
- nlzss's own `lzss3.decompress_bytes` decompresses each stream back to its
  archive, byte-identical to `nintendo_darc.arc` / `nintendo_sarc.sarc`.

Ludared is not involved in any of those checks.

## What the tests pin

`src/testing/codecs/e2e/nintendo/{darc,sarc,lz10,lz11}.rs` extend the existing
`CodecE2E` properties with these fixtures:

- **`decode_golden`** — extracted paths, sizes and SHA-256s, plus the recorded
  metadata artifacts (`DARC.json`, `SARC.json`) and, for the streams, the
  decompressed archive hash. The expectations come from the tool output and the
  source content, never from Ludared's encoder.
- **`bit_exact`** — both archives rebuild byte-for-byte unchanged, including
  the SARC fixture's `0x77` bytes of trailing zero padding.
- **`rebuilds_independent_streams_equivalently`** — the rebuilt LZ10/LZ11
  stream is decompressed again and compared with the original archive.
  Byte equality with the nlzss stream is reported by the test but not required:
  LZ10/LZ11 have many valid encodings, so a different stream is not an
  incompatibility. Ludared's matcher produces a different (valid) encoding for
  all four streams: the DARC streams are 4 bytes shorter and the SARC streams
  are the same length but differ in their tokens.

`DARC.json` decodes to 1097 bytes (`e04e9474…`) and `SARC.json` to 1139 bytes
(`200008b1…`); those hashes are pinned in the `decode_golden` fixtures.

## Regenerating

The tools are not vendored. To install the pinned revisions locally and build
darctool:

```sh
mkdir -p DO_NOT_COMMIT/TOOLS
git clone https://github.com/dnasdw/darctool.git DO_NOT_COMMIT/TOOLS/dnasdw_darctool
git -C DO_NOT_COMMIT/TOOLS/dnasdw_darctool checkout 004ca96
git -C DO_NOT_COMMIT/TOOLS/dnasdw_darctool submodule update --init --recursive
cmake -S DO_NOT_COMMIT/TOOLS/dnasdw_darctool \
    -B DO_NOT_COMMIT/TOOLS/dnasdw_darctool/build \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build DO_NOT_COMMIT/TOOLS/dnasdw_darctool/build --config Release

git clone https://github.com/Tyulis/3DSkit.git DO_NOT_COMMIT/TOOLS/3DSkit
git -C DO_NOT_COMMIT/TOOLS/3DSkit checkout 2fb6077

git clone https://github.com/magical/nlzss.git DO_NOT_COMMIT/TOOLS/nlzss
git -C DO_NOT_COMMIT/TOOLS/nlzss checkout f27414f
```

Then generate or check the fixtures with:

```sh
python3 src/testing/fixtures/nintendo/generate_nintendo_archives.py --tools DO_NOT_COMMIT/TOOLS
python3 src/testing/fixtures/nintendo/generate_nintendo_archives.py --tools DO_NOT_COMMIT/TOOLS --check
```

`--check` rebuilds everything in memory and fails if a committed fixture would
change.

### Reverse-direction interoperability

`--verify-interop` closes the loop the fixtures open: it decodes every fixture
with Ludared, rebuilds it unchanged, decompresses streams with nlzss, extracts
the archives with dnasdw's tool / 3DSkit, and compares every path and file with
the source tree above. It then repeats the check after replacing `hi.txt` with
different deterministic contents through Ludared's override workflow. It needs
the same third-party tools plus a built binary:

```sh
cargo build
python3 src/testing/fixtures/nintendo/generate_nintendo_archives.py \
    --tools DO_NOT_COMMIT/TOOLS --verify-interop --ludared target/debug/ludared
```

Adding or removing a file is not covered there: the CLI can only override
artifacts the decode tree already contains (`project::add_override` resolves the
virtual path against the manifest), and a metadata edit that adds or drops a
node leaves an artifact unmatched, which the codecs refuse. Doing that through
the CLI would need a new operation, not a new test.

## Interoperability notes

- **yellows8/darctool** (`darctool.c`, `96e0b1a`) builds archives with a `.`
  entry too, but writes parent index `1` for *every* folder. Its output is
  therefore wrong for nested trees: dnasdw's extractor mis-nests `nested`, and
  Ludared rejects the archive. It was not used. 3DSkit's DARC packer
  (`pack/DARC.py`) writes the same constant `1`, so DARC comes from dnasdw's
  tool, which computes real parent indices.
- **LITTOMA/darctool** (Python 2.7) writes parent index `0` for every folder
  and needs source patches to run on Python 3; it was not used either.
- 3DSkit's SARC packer writes `has_name` (0x01) attributes and pads the name
  table to 0x80, both of which Ludared preserves on rebuild.
- **SARC trailing padding round-trips.** 3DSkit pads the file to a 0x80
  boundary after the last node but excludes that padding from the header's
  total size (`0x989` declared, `0xa00` on disk). An all-zero tail is recorded
  as `trailing_zeros` (its length) instead of as a hex string of zeros, so a
  padded archive stays compact in metadata and still rebuilds byte-for-byte.
  A tail with any non-zero byte keeps its exact bytes in `trailing`, and both
  fields are absent when the archive ends at its declared size.
