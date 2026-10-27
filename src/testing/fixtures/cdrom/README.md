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
