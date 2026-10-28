# Test fixtures

## `mode1_golden.bin`

A synthetic four-sector CD-ROM Mode 1/2352 image, 9408 bytes. Its 8192 user
data bytes are public-domain synthetic data: two zero-filled sectors, one
sector of `0xFF`, and one byte-pattern sector. The raw sectors were generated
by the independent open-source `cue-2-ccd` tool, not by Ludared. The fixture
contains no third-party or proprietary data and is dedicated to the public
domain under CC0 1.0.

- **File SHA-256:** `fb1fbb777a57ad1c890c19b513589bc5708726734cb0eb3fc68f1a9ed1e6f7ad`
- **Decoded user-data SHA-256:** `4bf9580f28aab520cc05fd341d4147c9abc7c9b95d7c1cc5dfdd7ce54e85ee6b`

### Independent generation

Generated with [Derek Pascarella's `cue-2-ccd`](https://github.com/DerekPascarella/cue-2-ccd),
GPL-3.0, upstream release tag `1.1.0` (source commit `a85ad25`). This tool
converts a `MODE1/2048` CUE track into raw 2352-byte sectors with sync, MSF
header, EDC, and ECC. The release source builds with .NET 8.0. Building the
tag directly prints assembly version `1.0.0` because the project does not set
assembly version metadata; the source and release tag used here are `1.1.0`.

From the Ludared repository root, these commands fetch the `1.1.0` source and
reproduce the fixture:

```sh
git clone --depth 1 --branch 1.1.0 https://github.com/DerekPascarella/cue-2-ccd.git /tmp/cue-2-ccd
work=$(mktemp -d)
mkdir -p "$work/out"
python3 - "$work" <<'PY'
import sys
from pathlib import Path

root = Path(sys.argv[1])
data = bytearray(8192)
data[2048:4096] = b"\xff" * 2048
data[4096:] = bytes((index * 37 + 13) & 0xff for index in range(4096))
(root / "payload.bin").write_bytes(data)
(root / "payload.cue").write_text(
    'FILE "payload.bin" BINARY\n'
    '  TRACK 01 MODE1/2048\n'
    '    INDEX 01 00:00:00\n'
)
PY
dotnet build /tmp/cue-2-ccd/cue-2-ccd.csproj -c Release
dotnet /tmp/cue-2-ccd/bin/Release/net8.0/cue-2-ccd.dll "$work/payload.cue" "$work/out"
cp "$work/out/payload.img" src/testing/fixtures/cdrom/mode1_golden.bin
```

The generated payload hash is the decoded user-data hash above; the generated
`payload.img` hash is the fixture hash above. Ludared's E2E checks verify both
the decoded bytes and byte-exact reconstruction against this independently
generated image.

## `mode2_xa_golden.bin`

A 30-sector (70,560-byte) synthetic ISO9660/XA data disc. It contains three
ordinary files with deterministic byte-pattern contents, an ISO9660 directory
tree and path tables, and standard 14-byte XA System Use Information in the
directory records. It has no VCD/SVCD/CD-i directory structure or
application-specific metadata, and contains no MPEG multiplex. All file and
Form 2 extension data is generated
by the fixture script and is dedicated to the public domain under CC0 1.0.

- **Raw BIN SHA-256:** `4d8baeb16c66cfb4df7994afe5047a546351a3877ac7b428352ca9b0fb9b4646`
- **Cooked ISO SHA-256:** `8659913b31c6fb3c3b47ba6b6a1f28e7a50a462dfc7bd7caed85f147a0198f33`
- **Independent ISO metadata SHA-256:** `4025e86d7968d4e96f9496b43b46c6a46c6645b14feeeb57168fbcf8aea20618`

### Tools and reproduction

The ISO9660 filesystem comes from cdrtools `mkisofs` 3.02a09 using its generic
`-xa` option. Its source archive is
`https://sourceforge.net/projects/cdrtools/files/alpha/cdrtools-3.02a09.tar.bz2/download`
(SHA-256 `aa28438f458ef3f314b79f2029db27679dae1d5ffe1569b6de57742511915e81`).
GNU VCDImager 2.0.1 is used only as an independent raw Mode 2 XA sector
encoder through its `_vcd_make_mode2` API; the fixture is not authored as a
Video CD. Its source archive is
`https://ftp.gnu.org/gnu/vcdimager/vcdimager-2.0.1.tar.gz` (SHA-256
`67515fefb9829d054beae40f3e840309be60cda7d68753cafdd526727758f67a`). The
encoder routine writes sector sync, BCD MSF header, Mode 2 subheader, EDC and
ECC. `generate_mode2_xa.py` requires both pinned tool versions and generates
all file contents, XA metadata, sector manifest, and expected ISO metadata.

Build VCDImager 2.0.1 (with its libcdio/libiso9660/libpopt/libxml2 development
dependencies) and cdrtools 3.02a09. Then run from the repository root:

```sh
python3 src/testing/fixtures/cdrom/generate_mode2_xa.py \
  --vcdimager-source /tmp/vcdimager-2.0.1 \
  --output-dir src/testing/fixtures/cdrom
```

The generator needs `mkisofs` 3.02a09 and `pkg-config libcdio` on `PATH`. It
creates the cooked ISO with `-iso-level 1 -no-pad -xa`, normalizes volatile
PVD fields and the non-filesystem producer record, adds the XA file numbers
and Form 2 file attribute, then encodes
each 2048-byte sector payload with VCDImager's sector API. The ISO directory
records and expected `ISO9660.json` are parsed by the independent Python
generator; `isoinfo -R -l` independently lists the same directory tree.

### Source files and sector layout

The three source files are deterministic:

| Path | Source bytes | Size | Source SHA-256 |
| --- | --- | ---: | --- |
| `PLAIN.TXT` | `(i * 11 + 5) % 256` | 3000 | `d27ce922a6279c04fa85c64fae72e67d8339d15eaaa81e4b733f6ce674e38bc2` |
| `XA/FORM1.BIN` | `(i * 13 + 7) % 256` | 4096 | `6c0294b06b53f1e89f9978a127bf9ee4182239125570f700aa47f39d79a6d278` |
| `XA/FORM2.BIN` | `(i * 17 + 9) % 256` | 2048 | `880a3e3a3085790074f2c479f4698c7295d85523a5c7f417032a702fcf829fbf` |

The extra 276 bytes in the Form 2 sector use `(i * 19 + 11) % 256`, SHA-256
`f0230637215fe6ba17939bde2583cbe459ab79249d49319fe26c0e29fbe4d7eb`. ISO
Data Length remains 2048 bytes for the Form 2 file, while Ludared's XA
contract appends the complete 2324-byte sector payload to the decoded artifact.
The fixture intentionally uses separate all-Form-1 and all-Form-2 files:
the XA directory attribute identifies one form per file, so a mixed-form file
would contradict the XA metadata.

| LSN | Use | Form | XA subheader (file, channel, submode, coding) |
| ---: | --- | --- | --- |
| 0–15 | System area | Form 1 | default data submode `08` |
| 16 | Primary Volume Descriptor | Form 1 | `00,00,09,00` |
| 17 | Volume descriptor terminator | Form 1 | final/default `89` |
| 18 | fixed cdrtools producer record | Form 1 | default `08` |
| 19–20 | little-endian path table | Form 1 | `89` on final sector |
| 21–22 | big-endian path table | Form 1 | `89` on final sector |
| 23 | root directory | Form 1 | `89` |
| 24 | `XA` directory | Form 1 | `89` |
| 25–26 | `PLAIN.TXT` | Form 1 | file 0/channel 0, `08` then `89` |
| 27–28 | `XA/FORM1.BIN` | Form 1 | file 2/channel 3, `08` then `89` |
| 29 | `XA/FORM2.BIN` | Form 2 | file 3/channel 1, `A9` |

This gives 29 Form 1 sectors and one Form 2 sector. The `PLAIN.TXT` XA
directory entry uses the cdrtools default Form 1 attributes and file number 0,
so it has no `.XA` sidecar. `FORM1.BIN` has Form 1 XA attributes and file
number 2; `FORM2.BIN` has Form 2 XA attributes and file number 3. Both use
non-default XA channel values, so their `.XA` sidecars are expected.

### Expected decoded artifacts

The `raw` mode returns `data.bin` (the cooked 61,440-byte ISO), a 280-byte
`xa_data.bin` record for LSN 29 (little-endian sector index plus the 276-byte
Form 2 extension), and 120 bytes of subheaders (four bytes per sector).
The `iso9660` mode returns `XA.json`, independently generated `ISO9660.json`,
the three files, and sidecars `XA/FORM1.BIN.XA` (`03 08 00 03 89 00`) and
`XA/FORM2.BIN.XA` (`01 A9 00`). There is no sidecar for `PLAIN.TXT`.

`XA.json` is exactly:

```json
{
  "file_number": {
    "XA/FORM1.BIN": 2,
    "XA/FORM2.BIN": 3
  }
}
```

Expected decoded artifacts:

| Mode | Artifact | Size | SHA-256 |
| --- | --- | ---: | --- |
| raw | `data.bin` | 61440 | `8659913b31c6fb3c3b47ba6b6a1f28e7a50a462dfc7bd7caed85f147a0198f33` |
| raw | `xa_data.bin` | 280 | `cfc047774e814268cf177e172716d3a040e68abb3ee3b1d5ae7fe17c7c7bc932` |
| raw | `xa_subheaders.bin` | 120 | `b21ccdbba33170681ab5670532b18036cc531a709527fda8e6419f35c3c71117` |
| iso9660 | `XA.json` | 71 | `fc3938de9ec230228efe615a6793ebbc73623863a54efb4c1e11c57758ef2513` |
| iso9660 | `ISO9660.json` | 4148 | `4025e86d7968d4e96f9496b43b46c6a46c6645b14feeeb57168fbcf8aea20618` |
| iso9660 | `PLAIN.TXT` | 3000 | `d27ce922a6279c04fa85c64fae72e67d8339d15eaaa81e4b733f6ce674e38bc2` |
| iso9660 | `XA/FORM1.BIN` | 4096 | `6c0294b06b53f1e89f9978a127bf9ee4182239125570f700aa47f39d79a6d278` |
| iso9660 | `XA/FORM2.BIN` | 2324 | `0a60471b6924903e2a8f0e33d9dc156997b56579c2fb4f1f287abd7dc23fd467` |
| iso9660 | `XA/FORM1.BIN.XA` | 6 | `09ba661a81cb2e05f4cf9f66f7e49980d68e67f352be3d22d2c5630e8996bec4` |
| iso9660 | `XA/FORM2.BIN.XA` | 3 | `d68ac288c7f961c40703dcf301d84e713fe6e65bd6ccbbadffb27dd7093f2df2` |

Expected file hashes come from the generated source files plus the separately
specified Form 2 extension bytes. The ISO metadata JSON is built directly from
the independently mastered ISO's PVD, path tables, directory records, and XA
System Use Information, not from Ludared. The E2E encode inputs are explicit
source artifacts in `mode2_xa.rs`; they do not decode the BIN fixture first.

## `iso9660_golden.iso`

A plain ISO 9660 image mastered by cdrtools `mkisofs` (not by anything in this
repository), 71680 bytes:

```
mkisofs -quiet -no-pad -iso-level 1 -VGOLDEN -o iso9660_golden.iso src/
```

`iso9660_golden.sh` writes the sources and runs that command. The sources are
nine deterministic files chosen to cover the decode's edges: a file that is not
a whole block (1000 bytes), one of exactly one block (2048), one crossing into
a second block (2049), an empty file, an extensionless file (`NOTHING`, which
cdrtools stores as `NOTHING.;1`), a mixed-case name (`mixed.txt`, uppercased
by Level 1 rules), two directories holding the same basename (`DUP.BIN`), and a
nested file. `-no-pad` keeps the image small; the zero padding mkisofs adds by
default is exercised by the `mkisofs`-driven unit tests instead.

- **File SHA-256:** `3626b0e829dbc06c88d0be059336194de56b6e587e540a30369a18e36e47a3cf`

The image holds no game data and nothing copyrighted. It is generated from the
public-domain sources in `iso9660_golden.sh`.

mkisofs stamps the time it ran into the image, so a regenerated image differs
from the committed one; the committed bytes are the fixture.

### Independent validation

Neither the fixture nor its expected hashes were produced by running the codec
under test:

- The fixture comes from `mkisofs`, and every file's expected SHA-256 was
  computed from the source bytes `mkisofs` was handed (the same values
  `iso9660_golden.sh` writes), before the codec ever read the image.
- libarchive (`bsdtar -tvf` / `bsdtar -xOf`) lists the image's files in the
  order `src/testing/codecs/e2e/cdrom/iso9660.rs` records them, with the same
  sizes, and every extracted file hashes to its source hash. `bsdtar` is an
  independent implementation and is not a test dependency.
- `REBUILT_SHA256` is the SHA-256 of the committed image itself: the unchanged
  rebuild is required to be the input's own bytes, so the golden expectation
  is the fixture, not a codec output.

The one value recorded from a run of the codec is `ISO9660.json`'s hash, which
pins the metadata artifact's format, and the modified-rebuild hash, which pins
what a grown `DATA.BIN` rebuilds to. Both are documented as recorded there;
the modified rebuild was verified file by file against the unchanged decode
before it was pinned (only `DATA.BIN` changed, and it holds the grown bytes).

## `udf_golden.iso`

A synthetic UDF 1.02 bridge image, 563200 bytes. `udf_golden.sh` creates two
fixed source files containing only generated text and byte patterns, then uses
cdrtools `mkisofs` 3.02a09 with `-udf` to master both the UDF filesystem and its
parallel ISO 9660 filesystem. `DATA.BIN` contains 1000 bytes generated by
`(index * 17 + 9) % 256`; `DIR_A/NOTE.TXT` contains
`Synthetic UDF bridge fixture.\n`. The source files contain no third-party or
commercial data and are dedicated to the public domain under CC0 1.0.

The generator adjusts `DATA.BIN`'s UDF recorded allocation extent to reserve
one full 2048-byte block while retaining its 1000-byte information length, and
recomputes the File Entry descriptor CRC and tag checksum. This models a file
with unused space in its existing extent, so the E2E growth case changes it to
1200 bytes without moving it or allocating another block. The image is larger
than 500 KB because the UDF anchor is at logical block 256; 257 blocks alone
require 526336 bytes.

- **File SHA-256:** `2f5e6c4d7770f66be4e6967ce1d755d179071cae26776acb064ea2fec74f2d5d`
- **Decoded `DATA.BIN` SHA-256:** `0e57164c5bec5951ce2e9f0ad8c9c3637d50e818fa0a6403ffdb85a886f2c6e4`
- **Decoded `DIR_A/NOTE.TXT` SHA-256:** `d1927e49f19cd9432b8e8ca2df0cfb1cc88966c07647d0e3d16df3c2abdd3b3a`

The exact fixture bytes are committed. `mkisofs` records build-time values in
UDF descriptors and its implementation-use area, so rerunning the script
recreates the fixed file contents and structure but may produce different
image bytes. The committed image hash and E2E goldens pin the checked-in bytes.

Independent `bsdtar -tf` lists the ISO 9660 bridge view, and `bsdtar -xOf`
extracts both source files with the hashes above. The E2E decode golden checks
the UDF-root artifact set (`UDF.json`, `DATA.BIN`, and `DIR_A/NOTE.TXT`); there
is one artifact for each non-empty file shared by the two filesystems. The
canonical rebuild is bit-exact. The growth golden pins the rebuild after
`DATA.BIN` becomes 1200 bytes of `0xA5`.

- **Decoded `UDF.json` SHA-256:** `7928bf917cd7dac153d500a03b7f49e8c5cba5b92e5921de89f7d14c98470895`
- **Grown rebuild SHA-256:** `4975ba8d610147cf7e9fc6fa36a0b31fec862d72eb7aaa13b3ab241575ebfe92`
