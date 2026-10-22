#!/bin/sh
# Regenerates mkisofs_golden.iso from deterministic sources.
#
# mkisofs stamps the time it ran into the image, so a regenerated image is not
# byte-identical to the committed one: the golden hashes in
# src/testing/codecs/e2e/cdrom/iso9660.rs have to be re-pinned afterwards (and
# re-validated against bsdtar, as README.md describes).
set -eu

dir=$(dirname "$0")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/src/DIR_A" "$work/src/DIR_B"

python3 - "$work/src" <<'EOF'
import sys, os

src = sys.argv[1]

def write(path, data):
    with open(os.path.join(src, path), "wb") as handle:
        handle.write(data)

write("DATA.BIN", bytes((i * 7 + 3) % 256 for i in range(1000)))
write("EMPTY.BIN", b"")
write("EXACT.BIN", bytes((i * 13 + 5) % 256 for i in range(2048)))
write("OVER.BIN", bytes((i * 3 + 1) % 256 for i in range(2049)))
write("mixed.txt", b"mixed case content\n")
write("NOTHING", b"no extension here\n")
write("DIR_A/INNER.BIN", b"inner contents of DIR_A/INNER.BIN\n")
write("DIR_A/DUP.BIN", b"first copy\n")
write("DIR_B/DUP.BIN", b"second copy\n")
EOF

mkisofs -quiet -no-pad -iso-level 1 -VGOLDEN -o "$dir/mkisofs_golden.iso" "$work/src"
