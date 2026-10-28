#!/usr/bin/env python3
"""Create the `nintendo_3ds_ncch.ncch` golden fixture.

This script is fixture-creation tooling, run once by hand. It is NOT part of
the build or the test suite: the Rust tests read the committed `.ncch` file
directly through `include_bytes!` and never invoke Python.

The image is entirely synthetic. Every byte is produced by the deterministic
rules below, so nothing here is game data, a Nintendo key, or any other
copyrighted material. The file is dedicated to the public domain (CC0 1.0).

NCCH layout produced (media unit 0x200):

    0x0000 .. 0x0200   NCCH header
    0x0200 .. 0x0600   exheader           (0x400 bytes)
    0x0600 .. 0x0A00   access descriptor   (0x400 bytes)
    0x0A00 .. 0x0C00   logo region        (0x200 bytes)
    0x0C00 .. 0x0E00   plain region       (0x200 bytes)
    0x0E00 .. 0x2E00   ExeFS              (0x2000 bytes)
    0x2E00 .. 0x3200   RomFS              (0x400 bytes)

    content_size = 0x3200 bytes (0x19 media units)
    flags[5]     = 0x03 (form type: executable)
    flags[7]     = 0x04 (NoCrypto: every region hash is over plaintext)

The region hashes follow the format: the logo hash covers the whole logo
region, the exheader hash covers `extended_header_size` bytes, and the ExeFS
and RomFS superblock hashes cover the first `hash_region_size` media units
of their region. The header RSA signature is a placeholder pattern: it cannot
be recomputed without Nintendo's private key and is carried verbatim.

Usage: python3 nintendo_3ds_ncch.py [output]     (defaults to the sibling file)
"""
import hashlib
import os
import struct
import sys

UNIT = 0x200
EXHEADER_SIZE = 0x400
ACCESS_DESCRIPTOR_SIZE = 0x400
IMAGE_SIZE = 0x3200

# (name, offset, size) of each region, in bytes.
EXHEADER = ("exheader", 0x0200, EXHEADER_SIZE)
ACCESS_DESCRIPTOR = ("access descriptor", 0x0600, ACCESS_DESCRIPTOR_SIZE)
LOGO_REGION = ("logo region", 0x0A00, 0x200)
PLAIN_REGION = ("plain region", 0x0C00, 0x200)
EXEFS = ("ExeFS", 0x0E00, 0x2000)
ROMFS = ("RomFS", 0x2E00, 0x400)

PRODUCT_CODE = b"LUDARED-NCCH-FIX"
PROGRAM_ID = 0x0004_0000_8E50_0000
PARTITION_ID = 0x0004_0000_1234_5678

# (index, offset_units, size_units) of the unit-based regions.
UNIT_REGIONS = [
    ("plain region", PLAIN_REGION[1] // UNIT, PLAIN_REGION[2] // UNIT),
    ("logo region", LOGO_REGION[1] // UNIT, LOGO_REGION[2] // UNIT),
    ("ExeFS", EXEFS[1] // UNIT, EXEFS[2] // UNIT),
    ("RomFS", ROMFS[1] // UNIT, ROMFS[2] // UNIT),
]

# How many media units of each region the superblock hashes cover.
EXEFS_HASH_REGION_UNITS = 1
ROMFS_HASH_REGION_UNITS = 1


def pattern(seed, length):
    """A deterministic byte pattern: byte i is (seed + i * step) mod 256."""
    step = seed | 1
    return bytes((seed + i * step) & 0xFF for i in range(length))


def region_bytes(name, offset, size):
    return pattern((sum(name.encode()) & 0xFF) or 1, size)


def header(regions, content_units):
    """Builds the header from the regions it describes.

    `regions` maps each region's name to its bytes; the four unit-based
    regions sit at fixed offsets and take their size from their content.
    """
    head = bytearray(0x200)
    head[0x000:0x100] = pattern(0x55, 0x100)             # RSA-2048 signature (placeholder)
    head[0x100:0x104] = b"NCCH"
    head[0x104:0x108] = struct.pack("<I", content_units)
    head[0x108:0x110] = struct.pack("<Q", PARTITION_ID)
    head[0x110:0x112] = struct.pack("<H", 0x3030)        # maker code "00"
    head[0x112:0x114] = struct.pack("<H", 2)             # format version (CXI)
    head[0x114:0x118] = struct.pack("<I", 0)             # seed verify
    head[0x118:0x120] = struct.pack("<Q", PROGRAM_ID)
    head[0x130:0x150] = hashlib.sha256(regions["logo region"]).digest()
    head[0x150:0x160] = PRODUCT_CODE
    head[0x160:0x180] = hashlib.sha256(regions["exheader"]).digest()
    head[0x180:0x184] = struct.pack("<I", EXHEADER_SIZE)
    head[0x188:0x190] = bytes([0, 0, 0, 0, 0, 3, 0, 4])   # form executable, NoCrypto

    for name, base in [
        ("plain region", 0x190),
        ("logo region", 0x198),
        ("ExeFS", 0x1A0),
        ("RomFS", 0x1B0),
    ]:
        offset, size = REGION_OFFSETS[name], len(regions[name])
        head[base:base + 4] = struct.pack("<I", offset // UNIT)
        head[base + 4:base + 8] = struct.pack("<I", size // UNIT)

    head[0x1A8:0x1AC] = struct.pack("<I", EXEFS_HASH_REGION_UNITS)
    head[0x1B8:0x1BC] = struct.pack("<I", ROMFS_HASH_REGION_UNITS)
    head[0x1C0:0x1E0] = hashlib.sha256(
        regions["ExeFS"][: EXEFS_HASH_REGION_UNITS * UNIT]
    ).digest()
    head[0x1E0:0x200] = hashlib.sha256(
        regions["RomFS"][: ROMFS_HASH_REGION_UNITS * UNIT]
    ).digest()
    return bytes(head)


REGION_OFFSETS = {
    "exheader": EXHEADER[1],
    "access descriptor": ACCESS_DESCRIPTOR[1],
    "logo region": LOGO_REGION[1],
    "plain region": PLAIN_REGION[1],
    "ExeFS": EXEFS[1],
    "RomFS": ROMFS[1],
}


def build(content_units=IMAGE_SIZE // UNIT, exefs=None, romfs=None):
    regions = {
        "exheader": region_bytes(*EXHEADER),
        "access descriptor": region_bytes(*ACCESS_DESCRIPTOR),
        "logo region": region_bytes(*LOGO_REGION),
        "plain region": region_bytes(*PLAIN_REGION),
        "ExeFS": exefs if exefs is not None else region_bytes(*EXEFS),
        "RomFS": romfs if romfs is not None else region_bytes(*ROMFS),
    }

    image = bytearray()
    image += header(regions, content_units)
    # The extended header and the access descriptor sit first, then the regions
    # the header points at, each where its offset says.
    image += regions["exheader"]
    image += regions["access descriptor"]
    for name in ["logo region", "plain region", "ExeFS", "RomFS"]:
        image += regions[name]
    assert len(image) == content_units * UNIT, len(image)
    return bytes(image)


def artifacts():
    """The canonical decode's artifacts: name, size, SHA-256."""
    image = build()
    exheader = image[EXHEADER[1]:EXHEADER[1] + EXHEADER[2]]
    access_descriptor = image[
        ACCESS_DESCRIPTOR[1]:ACCESS_DESCRIPTOR[1] + ACCESS_DESCRIPTOR[2]
    ]
    logo = image[LOGO_REGION[1]:LOGO_REGION[1] + LOGO_REGION[2]]
    plain = image[PLAIN_REGION[1]:PLAIN_REGION[1] + PLAIN_REGION[2]]
    exefs = image[EXEFS[1]:EXEFS[1] + EXEFS[2]]
    romfs = image[ROMFS[1]:ROMFS[1] + ROMFS[2]]
    for name, data in [
        ("exheader.bin", exheader),
        ("access_descriptor.bin", access_descriptor),
        ("logo_region.bin", logo),
        ("plain_region.bin", plain),
        ("exefs.bin", exefs),
        ("romfs.bin", romfs),
    ]:
        print(f"  {name:24} {len(data):6} bytes   {hashlib.sha256(data).hexdigest()}")


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "nintendo_3ds_ncch.ncch")
    image = build()
    with open(out, "wb") as handle:
        handle.write(image)
    print(f"wrote {len(image)} bytes to {out}")
    print(f"sha256 = {hashlib.sha256(image).hexdigest()}")
    print()

    # A rebuilt image whose ExeFS was replaced with different bytes of the same
    # size: the ExeFS superblock hash is recomputed over the new content.
    replaced = build(exefs=pattern(0x99, EXEFS[2]))
    print(f"replaced ExeFS ({len(replaced)} bytes)")
    print(f"  sha256 = {hashlib.sha256(replaced).hexdigest()}")

    # A rebuilt image whose RomFS grew by four media units: the declared size is
    # raised to fit, and the RomFS hash still covers its unchanged first unit.
    grown = build(
        content_units=IMAGE_SIZE // UNIT + 4,
        romfs=region_bytes(*ROMFS) + pattern(0xA5, 4 * UNIT),
    )
    assert len(grown) == IMAGE_SIZE + 4 * UNIT, len(grown)
    print(f"grown RomFS ({len(grown)} bytes)")
    print(f"  sha256 = {hashlib.sha256(grown).hexdigest()}")
    print()

    print("decode artifacts of the canonical image:")
    artifacts()


if __name__ == "__main__":
    main()
