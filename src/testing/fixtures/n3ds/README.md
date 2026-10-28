# Test fixtures

## `nintendo_3ds_cci.3ds`

Hand-crafted Nintendo 3DS CCI (NCSD) test image: a 0x200 header, a card info region, two NCCH-looking partitions, and a trailing padding region, 49152 bytes in all.

```
0x0000 .. 0x0200   NCSD header
0x0200 .. 0x4000   card info region
0x4000 .. 0x6000   partition 0   (0x2000 bytes, 16 media units)
0x6000 .. 0xA000   partition 1   (0x4000 bytes, 32 media units)
0xA000 .. 0xC000   trailer       (0x2000 bytes of trailing padding)

image_size = 0xC000 bytes (0x60 media units)
media_id   = 0x0004000012345678
```

Every byte is synthetic: it comes from the deterministic rules in `nintendo_3ds_cci.py`, run once to produce the image. The file holds no game data, no Nintendo key and nothing else copyrighted. The image and the script are dedicated to the public domain under CC0 1.0.

- **File SHA-256:** `543a0b2fba6949d5941261a289742113de86bee83d9a7c13c9816a4ea28d3d5d`
- **Rebuilt SHA-256:** `543a0b2fba6949d5941261a289742113de86bee83d9a7c13c9816a4ea28d3d5d` (byte-for-byte identical: decoding and rebuilding the fixture is an identity)

### Independent validation

Performed when the fixture was created and repeated after the `image_size` policy change below, with [pyctr](https://github.com/ihaveamac/pyctr) 0.7.6 (`CCIReader`, `load_contents=False`). pyctr and [ctrtool](https://github.com/3DSGuy/Project_CTR) are independent implementations of the format and are **not** build or test dependencies: `nintendo_3ds_cci.py` needs only the standard library, and the Rust tests read the committed bytes directly and never shell out.

Neither the fixtures nor the expected hashes were produced by running the code under test: the fixtures come from `nintendo_3ds_cci.py`, and every SHA-256 below was pinned only after an independent tool agreed with it.

Both the fixture and the image the actual codec rebuilt from it were parsed by `pyctr.type.cci.CCIReader`. It reported the same header and partition layout for each:

```
media_id   = 0004000012345678
image_size = 0xc000
section -3 (Header):      offset=0x0    size=0x200
section -2 (CardInfo):    offset=0x200  size=0x1000
section -1 (DevInfo):     offset=0x1200 size=0x300
section +0 (Application): offset=0x4000 size=0x2000
section +1 (Manual):      offset=0x6000 size=0x4000
```

pyctr accepted both files as valid CCIs, and the rebuilt image was byte-identical to the fixture (`cmp` clean). The decode artifacts were hashed individually too; `../cci.rs` records those hashes (name, size, SHA-256) and the `decode_golden` fixture pins them:

```
NCSD.json         1467 bytes   fdcb174cb0f729e28c3da7b55db9ea8c2e615df532dbb04d61d4a782eb6fa9b5
cardinfo.bin      15872 bytes  cbc22f891cfbb9bbc70de30ce89b0d77e2a71d5343f93b7e51756f5dd9456970
partition0.ncch   8192 bytes   c288c7beda83fa7b1b200304a8e4c1d24720255542653dc1f044d18f8858b888
partition1.ncch   16384 bytes  d787991206e11e6fce6d9d3b8d72ce33f729ff6a4933e432ef90674e1414b0d0
trailer.bin       8192 bytes   459ff52e3cd5e24d71acfa09fc52384415f63424f1b73b84e65e2e2c812ea218
```

### Modified rebuild

A second golden case covers a *modified* rebuild: partition 0 is grown by 4 media units (0x800 bytes, filled with `0xA5`), and the metadata's `image_size` is raised from 0xC000 (0x60 units) to 0xC800 (0x64 units). The encoder lays the partitions out contiguously again and records a recalculated table:

```
0x0000 .. 0x4000   header + card info   (unchanged)
0x4000 .. 0x6800   partition 0          (0x2800 bytes, 20 media units; 0x800 appended)
0x6800 .. 0xA800   partition 1          (0x4000 bytes, 32 media units; unchanged data)
0xA800 .. 0xC800   trailer              (0x2000 bytes; preserved verbatim)

image_size = 0xC800 bytes (0x64 media units; raised in NCSD.json)
```

The trailing region is carried verbatim, so growing partition 0 pushes it past the *old* declared size. The encoder refuses that layout — it never truncates the trailer and never changes `image_size` on its own — and says so, naming `image_size` as the field to raise. Raising it is the one metadata change the case makes; the encoder then lays the partitions out contiguously and records the recalculated table. The result is exactly the declared size, with no room left over.

- **Modified rebuilt SHA-256:** `00c022a35eca1400bc02837e14f11f2a8ac141b8a0d47791791104f209c07dc5`
- **Modified rebuilt length:** `0xC800` bytes

The case is *not* a check that a grown rebuild happens to come out at the declared size: the encoder is pinned there by unit tests too (`refuses_a_growth_whose_trailing_region_no_longer_fits`, `refuses_to_grow_a_partition_past_the_declared_size`).

Independently validated with pyctr 0.7.6 the same way. The rebuild was produced through the CLI (`ludared build` after overriding `partition0.ncch` and editing `NCSD.json`) so the validation covers the codec as shipped, not just a test harness. pyctr parsed it and reported the recalculated sections:

```
media_id   = 0004000012345678
image_size = 0xc800
file size  = 0xc800
section -3 (Header):      offset=0x0    size=0x200
section -2 (CardInfo):    offset=0x200  size=0x1000
section -1 (DevInfo):     offset=0x1200 size=0x300
section +0 (Application): offset=0x4000 size=0x2800   sha256=ed393cc21a0d9ea5019d564e30a518605c251fbe4cd87d27dd32070f82d52aa9
section +1 (Manual):      offset=0x6800 size=0x4000   sha256=d787991206e11e6fce6d9d3b8d72ce33f729ff6a4933e432ef90674e1414b0d0
```

The card info region (`0x200..0x4000`) and the trailer (`0xA800..0xC800`) were confirmed byte-identical to their artifacts, and the CLI build hashed to the same value the automated test pins.

[ctrtool](https://github.com/3DSGuy/Project_CTR) 1.3.0 was used as a second, independent implementation for both images. ctrtool's CCI processing aborts on *any* real CCI whose partitions declare a filesystem (its virtual-filesystem snapshot only accepts `fs type = None`), so it was built from source with that construction made non-fatal; everything else is stock. It reports, for the canonical fixture and the modified rebuild respectively:

```
RomSize:                0x00000060 blocks (Used: 0xA000)
 Partition 0   Area: 0x00004000-0x00006000   FsType: 01   CryptoType: 00
 Partition 1   Area: 0x00006000-0x0000A000   FsType: 01   CryptoType: 00

RomSize:                0x00000064 blocks (Used: 0xA800)
 Partition 0   Area: 0x00004000-0x00006800   FsType: 01   CryptoType: 00
 Partition 1   Area: 0x00006800-0x0000A800   FsType: 01   CryptoType: 00
```

It accepted both — neither "content geometry exceeded media size" (used > declared) nor "Input stream too small" (file < used) was raised — and its declared sizes, used sizes and partition areas match the tables above, in its own media units.

### `image_size`: what the codec does, and why

`0x104` is the size of the NCSD image in media units. It bounds the *whole* image:

- **A decode refuses an image longer than its declared size.** A trimmed image may be shorter — its file simply ends before the declared size — but bytes past the declared size belong to no part of the image, and a rebuild could not put them back, so they are an error rather than a trailer.
- **An encode refuses a layout that no longer fits.** The trailing region is preserved verbatim and never truncated or padded, so a grown partition can push the image past the declared size. That is refused, with `image_size` named as the field to raise in `NCSD.json`.
- **The declared size is never changed implicitly.** Only an explicit edit to `image_size` changes it.

Real images are always a prefix of their declared size, so this rejects nothing a cartridge dump or a trim produces. It is deliberately stricter than either validator: pyctr reads the declared size without constraining the file, and ctrtool only requires `used <= declared` and `file >= used`, so both *accept* the old behaviour of emitting a longer file than the header describes. The format's own definition of the field makes that an invalid image, and the codec does not reproduce it.

### Round-trip guarantees

The golden coverage fixes what the encoder promises:

- **Deterministic encoding.** For a given set of artifacts the encoder always produces the same bytes; the modified case above is what pins the recalculated-layout path.
- **Byte-identical round-trips for the supported canonical layouts.** The unmodified fixture decodes and rebuilds to identical bytes, and the modified fixture rebuilds to a fixed, independently validated image.
- **No universal bit-exact guarantee.** Layouts outside these cases — unusual partition arrangements, other trailing regions, or edits to the header fields — are not promised to round-trip bit-exactly. A rebuild always recalculates the partition table and preserves every artifact's content.

### Where the golden tests live

The three end-to-end properties are declared in `src/testing/codecs/e2e/n3ds/cci.rs` — the codec's implementation of the `CodecE2E` trait, wired up by the `codec_e2e_tests!` macro. The generic fixture types, assertions and macro live in `src/testing/codecs/e2e/mod.rs`, and the reference values (hashes, sizes, header fields, partition tables) in `src/testing/fixtures/cci.rs`.

## `nintendo_3ds_bcfnt.bcfnt`

Synthetic Nintendo 3DS BCFNT v3 font: one `FINF` section, one `TGLP` section with a single 8x8 RGBA8 sheet, one `CWDH` section and one scan `CMAP` section, 399 bytes in all.

```
0x0000 .. 0x0014   CFNT header
0x0014 .. 0x0034   FINF
0x0034 .. 0x0054   TGLP header
0x0054 .. 0x0154   texture sheet 0 (8x8 RGBA8, 256 bytes)
0x0154 .. 0x016D   CWDH (3 entries)
0x016D .. 0x018F   CMAP (scan, 3 entries)
```

The sheets follow the TGLP header immediately: `0x54` is the earliest offset the section allows, so the canonical layout carries no padding at all.

Every byte comes from the deterministic rules in `nintendo_3ds_bcfnt.py`, run once to produce the file. The image holds no game data and nothing copyrighted. The image and the script are dedicated to the public domain under CC0 1.0.

- **File SHA-256:** `f1c2ea0e6954c0b453a2b2c8785017e4cc2d62d08d2953d96f49acafbefffad8`
- **Rebuilt SHA-256:** `f1c2ea0e6954c0b453a2b2c8785017e4cc2d62d08d2953d96f49acafbefffad8` (decoding and rebuilding the fixture is an identity)

The decoded artifacts are:

```
BCFNT.json          517 bytes   b6477d3a133adbdf1210d3e3f9ec3be3de55ab349f1344802c40a50c0b242f7e
CWDH.json           368 bytes   3fbaa8f80cf9eb3e9ab3cbd6fd416a37a9e4f346c888e59f37dd127be78c82b2
CMAP.json           357 bytes   8bf7c08e866e7bfac6564cefe79efce242055cae232231346464b3499c1eae56
texture0.png        118 bytes   b0cd9e4924c3c5a0c580514db60919160b624ee8fd63cd1abc4f897ac58df689
```

### Independent validation

`bcfnt.py` from 3dstool was used as the reference oracle, with [pypng](https://github.com/drj11/pypng) installed only in a temporary directory. It is **not** a build or test dependency; the Rust tests read the committed bytes directly and never shell out.

`python3 bcfnt.py -x -f font.bcfnt` accepted every BCFNT fixture and produced manifests whose `fontInfo`, `textureInfo`, `glyphWidths` and `glyphMap` agree with the Rust artifacts. Its extracted `sheet0.png` has the same dimensions and identical RGBA pixels as the Rust `texture0.png`; the PNG files themselves differ only in encoder compression and filtering.

The fixture generator writes the canonical layout itself rather than using `bcfnt.py -c`. The reference's create path has known defects that make it unsuitable as a fixture producer: it writes `end - 1` into the CWDH header, writes a count before direct/table CMAP data that its own parser does not read, and its big-endian BOM write is inverted. The Rust encoder follows the format described by 3dbrew and the reference *parser*: CWDH ends at `end`, direct CMAP payloads are just `indexOffset`, table payloads are just the `IndexTable`, and the BOM bytes are `FF FE` for little-endian and `FE FF` for big-endian.

### Modified rebuild

A second golden case covers a modified rebuild: the second CWDH entry's char width is raised from 6 to 42, and the second scan mapping (code `0x21`) is pointed at glyph 2 instead of glyph 1.

- **Modified rebuilt SHA-256:** `e232100f9a6b857dad0120f507c0f346567116a97c7b351baf84a2078708ed51`
- **Length:** 399 bytes (unchanged)

The case is exercised both directly (`is_encode_golden`) and through the real CLI (`rebuilds_a_bcfnt_font_through_an_override`), which decodes the source, overrides `CWDH.json`, edits the width in the workspace, and runs `ludared build`.

## `nintendo_3ds_bcfnt_padded.bcfnt`

The padded-layout companion reproduces the sheet placement seen in retail fonts: the TGLP header ends at `0x54`, but the sheets begin at `0x80`, leaving 44 zero bytes inside the section. `nintendo_3ds_bcfnt.py --data-offset 0x80` writes it; every sheet byte and every other section equals the canonical fixture.

```
0x0000 .. 0x0014   CFNT header
0x0014 .. 0x0034   FINF
0x0034 .. 0x0054   TGLP header
0x0054 .. 0x0080   zero gap (not an artifact)
0x0080 .. 0x0180   texture sheet 0 (8x8 RGBA8, 256 bytes)
0x0180 .. 0x0199   CWDH (3 entries)
0x0199 .. 0x01BB   CMAP (scan, 3 entries)
```

- **File SHA-256:** `f2356d96d0c502beadf040f58791745d7f709096e1c3eed0a96b99723b5a27ab`
- **Size:** 443 bytes
- **Rebuilt SHA-256:** `f1c2ea0e6954c0b453a2b2c8785017e4cc2d62d08d2953d96f49acafbefffad8` (the canonical fixture, 399 bytes)

The gap is not exposed as metadata, so decoding this file produces exactly the same artifacts as the canonical fixture and rebuilding it drops the gap, relocating `CWDH`, `CMAP` and the FINF pointers. The unit test `decodes_a_padded_layout_and_rebuilds_it_compactly` pins that behavior, and `rejects_nonzero_tglp_gap_bytes_instead_of_dropping_them` uses this fixture to prove a non-zero gap is refused rather than discarded.

## `nintendo_3ds_bcfnt_etc1.bcfnt`

The ETC1 companion uses the same FINF/CWDH/CMAP layout and a single 8x8 ETC1 sheet (32 bytes). It exists to exercise PNG decode, lossy recompression and the normal CLI workflow for a compressed format.

- **File SHA-256:** `5c5cd56fa0a899dd8825ccda50ded6281b4f529205657ccb795b0e67e1a6badd`
- **Size:** 175 bytes
- **Rebuilt SHA-256:** `5c5cd56fa0a899dd8825ccda50ded6281b4f529205657ccb795b0e67e1a6badd` for this simple synthetic sheet

The decoded artifacts are:

```
BCFNT.json          516 bytes   48e343ff3584a339b0727bf3aee988446c0be0b54002d2709eb4407800daf9a9
CWDH.json           368 bytes   3fbaa8f80cf9eb3e9ab3cbd6fd416a37a9e4f346c888e59f37dd127be78c82b2
CMAP.json           357 bytes   8bf7c08e866e7bfac6564cefe79efce242055cae232231346464b3499c1eae56
texture0.png        118 bytes   d851e4bea23058376cd552a42c83b6e7a2833e235ebceeeaf2cd93fac9a43cc2
```

### Round-trip guarantees

- **Bit-exact for uncompressed formats.** The canonical RGBA8 fixture and other linear formats reproduce their native bytes when no artifact changes; this is verified by `is_bit_exact`.
- **Deterministic.** Decoding and encoding the same artifacts always produces the same bytes, including the PNG artifacts and their recorded SHA-256s. The ETC1 encoder evaluates a fixed candidate order and has no randomness.
- **ETC1/ETC1A4 are lossy.** Recompression is not guaranteed to reproduce the original blocks. The synthetic ETC1 fixture happens to re-encode byte-for-byte because its blocks are simple, but the codec does not claim that in general.
- **Canonical section order.** Rebuilds place `TGLP`, then every `CWDH`, then every `CMAP`. Files with a different physical section order are decoded and rebuilt into this canonical order rather than preserved verbatim.
- **Compact TGLP placement.** The encoder writes sheet data immediately after the TGLP header (0x54 for these fixed headers), so a canonical font carries no gap bytes. The format documentation imposes no alignment on the sheet data offset, so none is invented: the earliest legal offset is used. Gap bytes are never an artifact, and non-zero gap or trailing bytes in inputs are rejected rather than silently discarded.
- **Preserved unknown data.** CWDH padding, CMAP padding, CMAP's unknown field, FINF's reserved byte, and the header magic/version/byte order are carried through.
- **Texture edits.** Each sheet has exactly one artifact. Every format except HILO8 uses an editable PNG, including ETC1 and ETC1A4. HILO8 stays native `.bin`.

### Where the golden tests live

The three end-to-end properties are declared in `src/testing/codecs/e2e/n3ds/bcfnt.rs` and wired up by the `codec_e2e_tests!` macro. The codec's unit tests live in `src/codecs/nintendo/font/bcfnt/tests.rs`.
