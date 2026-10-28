#!/usr/bin/env python3
"""Generate the synthetic Nintendo 3DS BCFNT test fixtures.

Each output is a small, canonical BCFNT v3 font: one FINF section, one TGLP
section with a single 8x8 sheet, one CWDH section and one scan CMAP section.
The default fixture uses RGBA8; ``--format etc1`` produces the ETC1 companion
used to exercise lossy texture recompression. ``--data-offset 0x80`` reproduces
the sheet placement observed in retail fonts, where the sheets do not follow
the TGLP header immediately. Every byte is deterministic and synthetic; no game
data is used.

The script writes to the path given on the command line, or next to itself.
"""

import argparse
import os
import struct

MAGIC = b"CFNT"
BOM = b"\xff\xfe"
HEADER_SIZE = 0x14
FINF_SIZE = 0x20
TGLP_SIZE = 0x20
CWDH_SIZE = 0x10
CMAP_SIZE = 0x14
VERSION = 0x03000000
TGLP_DATA_OFFSET = HEADER_SIZE + FINF_SIZE + TGLP_SIZE


def rounded(value):
    return 1 << (value - 1).bit_length()


def pixels():
    """An 8x8 RGBA image with distinct columns, rows and alpha bands."""
    output = []
    for y in range(8):
        for x in range(8):
            output.append(
                (
                    x * 32,
                    y * 32,
                    (x * 16 + y * 16) & 0xFF,
                    255 if (x + y) % 4 else 128,
                )
            )
    return output


def etc1_block(color1, color2):
    """A vertical-mode ETC1 block with two 4-bit base colors."""
    red1, green1, blue1 = color1
    red2, green2, blue2 = color2

    bits = (
        (red1 << 60)
        | (green1 << 52)
        | (blue1 << 44)
        | (red2 << 56)
        | (green2 << 48)
        | (blue2 << 40)
    )

    return struct.pack("<Q", bits)


def etc1_sheet():
    blocks = [
        ((15, 2, 2), (2, 15, 2)),
        ((2, 2, 15), (15, 15, 2)),
        ((15, 0, 15), (0, 15, 15)),
        ((8, 8, 8), (2, 2, 2)),
    ]

    return b"".join(etc1_block(first, second) for first, second in blocks)


def swizzle_rgba8(image, width, height):
    rounded_width = rounded(width)
    rounded_height = rounded(height)
    data = bytearray(rounded_width * rounded_height * 4)

    for tile_y in range(rounded_height // 8):
        for tile_x in range(rounded_width // 8):
            for block_y in range(2):
                for block_x in range(2):
                    for group_y in range(2):
                        for group_x in range(2):
                            for pixel_y in range(2):
                                for pixel_x in range(2):
                                    x = pixel_x + group_x * 2 + block_x * 4 + tile_x * 8
                                    y = pixel_y + group_y * 2 + block_y * 4 + tile_y * 8
                                    if x >= width or y >= height:
                                        continue

                                    data_x = pixel_x + group_x * 4 + block_x * 16 + tile_x * 64
                                    data_y = (
                                        pixel_y * 2
                                        + group_y * 8
                                        + block_y * 32
                                        + tile_y * rounded_width * 8
                                    )
                                    position = (data_x + data_y) * 4
                                    data[position : position + 4] = bytes(
                                        image[y * width + x]
                                    )

    return bytes(data)


def build(sheet=None, format_code=0, data_offset=TGLP_DATA_OFFSET):
    if sheet is None:
        sheet = swizzle_rgba8(pixels(), 8, 8)

    cwdh_entries = [
        (1, 0, 5),
        (0, 1, 6),
        (2, 2, 7),
    ]

    cwdh_size = CWDH_SIZE + len(cwdh_entries) * 3
    cmap_entries = [
        (0x20, 0),
        (0x21, 1),
        (0x22, 2),
    ]
    cmap_size = CMAP_SIZE + 2 + len(cmap_entries) * 4

    tglp_start = HEADER_SIZE + FINF_SIZE
    tglp_section_size = data_offset + len(sheet) - tglp_start
    cwdh_start = data_offset + len(sheet)
    cmap_start = cwdh_start + cwdh_size
    file_size = cmap_start + cmap_size
    section_count = 4

    out = bytearray()

    # CFNT header
    out += MAGIC
    out += BOM
    out += struct.pack("<H", HEADER_SIZE)
    out += struct.pack("<I", VERSION)
    out += struct.pack("<I", file_size)
    out += struct.pack("<I", section_count)

    # FINF
    out += b"FINF"
    out += struct.pack("<I", FINF_SIZE)
    out += struct.pack(
        "<BBHBBBBIIIBBBB",
        1,  # font type
        20,  # line feed
        0,  # alter char index
        1,  # default left
        6,  # default glyph width
        7,  # default char width
        1,  # encoding
        tglp_start + 8,
        cwdh_start + 8,
        cmap_start + 8,
        24,  # height
        24,  # width
        20,  # ascent
        0,  # reserved
    )

    # TGLP header, padding and sheet
    out += b"TGLP"
    out += struct.pack("<I", tglp_section_size)
    out += struct.pack(
        "<BBBB",
        8,  # cell width
        8,  # cell height
        6,  # baseline
        8,  # max character width
    )
    out += struct.pack("<I", len(sheet))
    out += struct.pack("<HH", 1, format_code)
    out += struct.pack("<HHHH", 1, 1, 8, 8)
    out += struct.pack("<I", data_offset)
    out += b"\x00" * (data_offset - len(out))
    out += sheet

    # CWDH
    out += b"CWDH"
    out += struct.pack("<I", cwdh_size)
    out += struct.pack("<HH", 0, len(cwdh_entries) - 1)
    out += struct.pack("<I", 0)
    for left, glyph, char_width in cwdh_entries:
        out += struct.pack("<bBB", left, glyph, char_width)

    # CMAP (scan)
    out += b"CMAP"
    out += struct.pack("<I", cmap_size)
    out += struct.pack("<HHHH", 0x20, 0x22, 2, 0x1234)
    out += struct.pack("<I", 0)
    out += struct.pack("<H", len(cmap_entries))
    for code, glyph in cmap_entries:
        out += struct.pack("<HH", code, glyph)

    if len(out) != file_size:
        raise SystemExit(f"generated {len(out)} bytes, expected {file_size}")

    return bytes(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "output",
        nargs="?",
        help="output path (default: beside this script)",
    )
    parser.add_argument(
        "--format",
        choices=("rgba8", "etc1"),
        default="rgba8",
        help="sheet format to generate",
    )
    parser.add_argument(
        "--data-offset",
        type=lambda value: int(value, 0),
        default=None,
        help=(
            "absolute TGLP sheet data offset for the RGBA8 fixture "
            f"(default: {TGLP_DATA_OFFSET:#x}, directly after the TGLP header)"
        ),
    )
    args = parser.parse_args()

    if args.data_offset is not None and args.format != "rgba8":
        parser.error("--data-offset only applies to the rgba8 fixture")

    directory = os.path.dirname(__file__)
    if args.format == "etc1":
        image = build(etc1_sheet(), 0x0C)
        default = os.path.join(directory, "nintendo_3ds_bcfnt_etc1.bcfnt")
    else:
        offset = TGLP_DATA_OFFSET if args.data_offset is None else args.data_offset
        if offset < TGLP_DATA_OFFSET:
            parser.error(
                f"--data-offset must leave room for the TGLP header ({TGLP_DATA_OFFSET:#x})"
            )

        image = build(data_offset=offset)
        name = (
            "nintendo_3ds_bcfnt.bcfnt"
            if offset == TGLP_DATA_OFFSET
            else "nintendo_3ds_bcfnt_padded.bcfnt"
        )
        default = os.path.join(directory, name)

    with open(args.output or default, "wb") as file:
        file.write(image)


if __name__ == "__main__":
    main()
