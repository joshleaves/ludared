#!/usr/bin/env python3
"""Create the `nintendo_3ds_cci.3ds` golden fixture.

This script is fixture-creation tooling, run once by hand. It is NOT part of the
build or the test suite: the Rust tests read the committed `.3ds` file directly
through `include_bytes!` and never invoke Python.

The image is entirely synthetic. Every byte is produced by the deterministic
rules below, so nothing here is game data, a Nintendo key, or any other
copyrighted material. The file is dedicated to the public domain (CC0 1.0).

NCSD layout produced (media unit 0x200):

    0x0000 .. 0x0200   NCSD header
    0x0200 .. 0x4000   card info region
    0x4000 .. 0x6000   partition 0   (0x2000 bytes, 16 media units)
    0x6000 .. 0xA000   partition 1   (0x4000 bytes, 32 media units)
    0xA000 .. 0xC000   trailer       (0x2000 bytes of trailing padding)

    image_size  = 0xC000 bytes = 0x60 media units
    media_id    = 0x0004000012345678

Each partition carries the 'NCCH' magic at 0x100, so detection reports `Likely`
(the declared capacity is small and is not one of the retail capacities, but the
partitions look like NCCH containers).

Usage: python3 nintendo_3ds_cci.py [output]     (defaults to the sibling file)
"""
import hashlib
import os
import struct
import sys

UNIT = 0x200
FIRST_PARTITION = 0x4000
TRAILER_START = 0xA000
IMAGE_SIZE = 0xC000

# (index, offset_units, size_units) of each partition.
PARTITIONS = [
    (0, 0x20, 0x10),
    (1, 0x30, 0x20),
]

CARD_INFO_BANNER = b"LUDARED CCI FIXTURE - CARD INFO (synthetic, CC0)"
PARTITION_BANNER = b"LUDARED CCI FIXTURE - PARTITION %d (synthetic, CC0)"


def pattern(seed, length):
    """A deterministic byte pattern: byte i is (seed + i * step) mod 256."""
    step = seed | 1
    return bytes((seed + i * step) & 0xFF for i in range(length))


def card_info():
    region = bytearray(pattern(0x11, FIRST_PARTITION - 0x200))
    region[0:len(CARD_INFO_BANNER)] = CARD_INFO_BANNER
    return bytes(region)


def partition(index, size):
    body = bytearray(pattern(0x22 + index * 0x11, size))
    banner = PARTITION_BANNER % index
    body[0:len(banner)] = banner
    body[0x100:0x104] = b"NCCH"
    return bytes(body)


def trailer():
    return pattern(0x44, IMAGE_SIZE - TRAILER_START)


def header():
    head = bytearray(0x200)
    head[0x000:0x100] = pattern(0x55, 0x100)          # RSA-2048 signature (bogus)
    head[0x100:0x104] = b"NCSD"
    head[0x104:0x108] = struct.pack("<I", IMAGE_SIZE // UNIT)
    head[0x108:0x110] = struct.pack("<Q", 0x0004_0000_1234_5678)
    head[0x110:0x118] = bytes([1, 1, 0, 0, 0, 0, 0, 0])  # partition fs types
    head[0x118:0x120] = bytes([0, 0, 0, 0, 0, 0, 0, 0])  # partition crypto types
    for index, offset, size in PARTITIONS:
        base = 0x120 + index * 8
        head[base:base + 4] = struct.pack("<I", offset)
        head[base + 4:base + 8] = struct.pack("<I", size)
    head[0x160:0x180] = pattern(0x66, 0x20)              # exheader SHA-256
    head[0x180:0x184] = struct.pack("<I", 0)             # additional header size
    head[0x184:0x188] = struct.pack("<I", 0)             # sector zero offset
    head[0x188:0x190] = bytes([0, 0, 0, 0, 1, 1, 0, 0])  # platform CTR, type Card1
    for index in range(8):
        base = 0x190 + index * 8
        head[base:base + 8] = struct.pack("<Q", 0x0100_0000_0000_0000 + index)
    head[0x1D0:0x200] = pattern(0x77, 0x30)              # reserved tail
    return bytes(head)


def build():
    image = bytearray()
    image += header()
    image += card_info()
    for index, offset, size in PARTITIONS:
        assert len(image) == offset * UNIT, (len(image), offset * UNIT)
        image += partition(index, size * UNIT)
    image += trailer()
    assert len(image) == IMAGE_SIZE, len(image)
    return bytes(image)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "nintendo_3ds_cci.3ds")
    image = build()
    with open(out, "wb") as handle:
        handle.write(image)
    print(f"wrote {len(image)} bytes to {out}")
    print(f"sha256 = {hashlib.sha256(image).hexdigest()}")


if __name__ == "__main__":
    main()
