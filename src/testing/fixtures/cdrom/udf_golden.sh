#!/bin/sh
# Regenerates udf_golden.iso from fixed public-domain source files.
#
# The source mtimes and ISO volume modification date are fixed. mkisofs also
# stamps build-time values into UDF descriptors, so regenerated image bytes
# differ; the committed image and its hashes are pinned in the E2E tests.
set -eu

dir=$(dirname "$0")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/src/DIR_A"

python3 - "$work/src" <<'EOF'
import os
import sys

src = sys.argv[1]

def write(path, data):
    target = os.path.join(src, path)
    with open(target, "wb") as handle:
        handle.write(data)
    os.utime(target, (1577836800, 1577836800))

write("DATA.BIN", bytes((i * 17 + 9) % 256 for i in range(1000)))
write("DIR_A/NOTE.TXT", b"Synthetic UDF bridge fixture.\n")
EOF

touch -t 202001010000 "$work/src" "$work/src/DIR_A"
mkisofs -quiet -no-pad -udf -iso-level 1 -VGOLDEN_UDF \
  -uid 0 -gid 0 -file-mode 0644 -dir-mode 0755 \
  -modification-date 20200101000000.00+0000 \
  -o "$dir/udf_golden.iso" "$work/src"

# Reserve the rest of DATA.BIN's existing logical block. Its information
# length remains 1000 bytes, while its recorded extent has room for 2048.
python3 - "$dir/udf_golden.iso" <<'EOF'
import struct
import sys
from pathlib import Path

image_path = Path(sys.argv[1])
image = bytearray(image_path.read_bytes())
block_size = 2048
matches = []

def crc16(data):
    crc = 0
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = (((crc << 1) ^ 0x1021) if crc & 0x8000 else (crc << 1)) & 0xffff
    return crc

for offset in range(0, len(image), block_size):
    entry = image[offset:offset + block_size]
    if len(entry) != block_size or struct.unpack_from("<H", entry, 0)[0] != 261:
        continue
    if entry[27] == 5 and struct.unpack_from("<Q", entry, 56)[0] == 1000:
        matches.append(offset)

if len(matches) != 1:
    raise SystemExit(f"expected one 1000-byte UDF File Entry, found {len(matches)}")

entry = image[matches[0]:matches[0] + block_size]
extended_attributes = struct.unpack_from("<I", entry, 168)[0]
allocation_length = struct.unpack_from("<I", entry, 172)[0]
allocation = 176 + extended_attributes
if allocation_length < 8:
    raise SystemExit("DATA.BIN has no supported allocation descriptor")
encoded_length = struct.unpack_from("<I", entry, allocation)[0]
if encoded_length & 0xc0000000:
    raise SystemExit("DATA.BIN does not use a recorded allocation extent")
struct.pack_into("<I", entry, allocation, 2048)

crc_length = struct.unpack_from("<H", entry, 10)[0]
struct.pack_into("<H", entry, 8, crc16(entry[16:16 + crc_length]))
entry[4] = sum(entry[:4] + entry[5:16]) & 0xff
image[matches[0]:matches[0] + block_size] = entry
image_path.write_bytes(image)
EOF
