# Test fixtures

## `mkisofs_golden.iso`

A plain ISO 9660 image mastered by cdrtools `mkisofs` (not by anything in this
repository), 71680 bytes:

```
mkisofs -quiet -no-pad -iso-level 1 -VGOLDEN -o mkisofs_golden.iso src/
```

`mkisofs_golden.sh` writes the sources and runs that command. The sources are
nine deterministic files chosen to cover the decode's edges: a file that is not
a whole block (1000 bytes), one of exactly one block (2048), one crossing into
a second block (2049), an empty file, an extensionless file (`NOTHING`, which
cdrtools stores as `NOTHING.;1`), a mixed-case name (`mixed.txt`, uppercased
by Level 1 rules), two directories holding the same basename (`DUP.BIN`), and a
nested file. `-no-pad` keeps the image small; the zero padding mkisofs adds by
default is exercised by the `mkisofs`-driven unit tests instead.

- **File SHA-256:** `3626b0e829dbc06c88d0be059336194de56b6e587e540a30369a18e36e47a3cf`

The image holds no game data and nothing copyrighted. It is generated from the
public-domain sources in `mkisofs_golden.sh`.

mkisofs stamps the time it ran into the image, so a regenerated image differs
from the committed one; the committed bytes are the fixture.

### Independent validation

Neither the fixture nor its expected hashes were produced by running the codec
under test:

- The fixture comes from `mkisofs`, and every file's expected SHA-256 was
  computed from the source bytes `mkisofs` was handed (the same values
  `mkisofs_golden.sh` writes), before the codec ever read the image.
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
