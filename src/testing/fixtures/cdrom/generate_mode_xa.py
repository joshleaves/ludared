#!/usr/bin/env python3
"""Build a generic ISO9660/XA fixture using cdrtools and GNU VCDImager's sector API."""

import argparse
import json
import math
import os
import struct
import subprocess
import tempfile
from pathlib import Path

BLOCK = 2048
EXTRA = 276
FIXED_TIME = 1793059200
FIXED_DATE = b"2026102700000000\0"


def read32le(data, offset):
    return struct.unpack_from("<I", data, offset)[0]


def list_directory(image, extent, length):
    data = image[extent * BLOCK:extent * BLOCK + length]
    entries = []
    offset = 0
    while offset < len(data):
        size = data[offset]
        if size == 0:
            offset = ((offset // BLOCK) + 1) * BLOCK
            continue
        record = data[offset:offset + size]
        name_length = record[32]
        identifier = record[33:33 + name_length]
        name = identifier.decode("ascii") if identifier not in (b"\0", b"\1") else identifier
        system_use_offset = 33 + name_length + (name_length % 2 == 0)
        entries.append({
            "name": name,
            "identifier": identifier,
            "record_offset": extent * BLOCK + offset,
            "extent": read32le(record, 2),
            "length": read32le(record, 10),
            "flags": record[25],
            "date": record[18:25].hex(),
            "system_use": record[system_use_offset:].hex(),
            "pad": "",
        })
        offset += size
    return entries


def scan_tree(image, path, extent, length, dirs, files):
    records = list_directory(image, extent, length)
    dirs.append({"path": path, "extent": extent, "length": length, "records": records})
    for record in records:
        if record["name"] in (b"\0", b"\1"):
            continue
        name = record["name"].split(";", 1)[0]
        child = f"{path}/{name}" if path else name
        if record["flags"] & 2:
            scan_tree(image, child, record["extent"], record["length"], dirs, files)
        else:
            files[child] = record


def form2_extra():
    return bytes((i * 19 + 11) & 255 for i in range(EXTRA))


def read_path_table(image, sector, size):
    table = image[sector * BLOCK:sector * BLOCK + size]
    entries = []
    offset = 0
    while offset < len(table):
        name_length = table[offset]
        entry_size = 8 + name_length + (name_length & 1)
        name_bytes = table[offset + 8:offset + 8 + name_length]
        entries.append({
            "name": name_bytes.decode("ascii") if name_bytes else "\0",
            "extent": read32le(table, offset + 2),
            "parent": struct.unpack_from("<H", table, offset + 6)[0],
        })
        offset += entry_size
    return entries


def metadata_json(image, files, dirs):
    pvd = image[16 * BLOCK:17 * BLOCK]
    volume_id = pvd[40:72].decode("ascii").rstrip(" ")
    text = lambda start, width: pvd[start:start + width].decode("ascii").rstrip(" ")
    primary = {
        "system_identifier": text(8, 32),
        "application_identifier": text(574, 128),
        "application_use": pvd[883:1395].hex(),
    }
    for key, offset in [
        ("volume_creation_date", 813),
        ("volume_modification_date", 830),
        ("volume_expiration_date", 847),
        ("volume_effective_date", 864),
    ]:
        value = pvd[offset:offset + 17]
        if any(value):
            primary[key] = value.hex()

    path_size = read32le(pvd, 132)
    l_sector = read32le(pvd, 140)
    m_sector = int.from_bytes(pvd[148:152], "big")
    opaque = [
        {"offset": 17 * BLOCK, "data": image[17 * BLOCK:17 * BLOCK + 7].hex()},
        {
            "offset": 18 * BLOCK,
            "data": image[18 * BLOCK:19 * BLOCK].rstrip(b"\0").hex(),
        },
    ]
    directories = []
    for directory in dirs:
        records = []
        for entry in directory["records"]:
            item = {
                "name": "\0" if entry["identifier"] == b"\0" else "\1" if entry["identifier"] == b"\1" else entry["name"],
                "extent": entry["extent"],
                "length": entry["length"],
            }
            if entry["flags"]:
                item["flags"] = entry["flags"]
            item["date"] = entry["date"]
            if entry["system_use"]:
                item["system_use"] = entry["system_use"]
            records.append(item)
        directories.append({
            "path": directory["path"],
            "extent": directory["extent"],
            "length": directory["length"],
            "records": records,
        })
    metadata = {
        "volume_identifier": volume_id,
        "file_order": [name for name, _ in sorted(files.items(), key=lambda item: item[1]["extent"])],
        "layout": {
            "image_sectors": len(image) // BLOCK,
            "volume_space_size": read32le(pvd, 80),
            "opaque": opaque,
            "primary": primary,
            "path_table": {
                "l_sector": l_sector,
                "m_sector": m_sector,
                "entries": read_path_table(image, l_sector, path_size),
            },
            "directories": directories,
        },
    }
    return json.dumps(metadata, indent=2, ensure_ascii=True).encode("ascii")


def write_source_files(source):
    files = {
        "PLAIN.TXT": bytes((i * 11 + 5) & 255 for i in range(3000)),
        "XA/FORM1.BIN": bytes((i * 13 + 7) & 255 for i in range(4096)),
        "XA/FORM2.BIN": bytes((i * 17 + 9) & 255 for i in range(2048)),
    }
    for name, data in files.items():
        target = source / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    for path in [source, source / "XA", *(source / name for name in files)]:
        os.utime(path, (FIXED_TIME, FIXED_TIME))
    return files


def make_manifest(image, files):
    if len(image) % BLOCK:
        raise RuntimeError("mkisofs emitted a partial logical block")
    pvd = 16 * BLOCK
    if image[pvd] != 1 or image[pvd + 1:pvd + 6] != b"CD001":
        raise RuntimeError("missing ISO9660 Primary Volume Descriptor")

    # Normalize PVD dates and the non-filesystem producer record where
    # mkisofs writes its build time and command line.
    for offset in (813, 830, 864):
        image[pvd + offset:pvd + offset + 17] = FIXED_DATE
    image[pvd + 847:pvd + 864] = bytes(17)
    image[pvd + 883:pvd + 1395] = b" " * 512
    image[pvd + 1024:pvd + 1032] = b"CD-XA001"
    producer_record = b"MKI synthetic ISO9660/XA fixture\ncdrtools mkisofs 3.02a09\n"
    image[18 * BLOCK:19 * BLOCK] = producer_record.ljust(BLOCK, b"\0")

    root_record = image[pvd + 156:pvd + 190]
    dirs, extents = [], {}
    scan_tree(
        image,
        "",
        read32le(root_record, 2),
        read32le(root_record, 10),
        dirs,
        extents,
    )
    if set(extents) != set(files):
        raise RuntimeError(f"unexpected ISO file set: {sorted(extents)}")
    expected_sizes = {name: len(data) for name, data in files.items()}
    if {name: record["length"] for name, record in extents.items()} != expected_sizes:
        raise RuntimeError("mkisofs file lengths differ from their source files")

    modes = {
        "PLAIN.TXT": (0, 0, [0x08, 0x89]),
        "XA/FORM1.BIN": (2, 3, [0x08, 0x89]),
        "XA/FORM2.BIN": (3, 1, [0xA9]),
    }
    # cdrtools -xa supplies standard 14-byte XA directory records. Keep
    # its permissions and Form 1/2 flags, and set per-file numbers from the
    # physical subheaders below.
    for path, (number, _channel, _submodes) in modes.items():
        record = extents[path]
        record_offset = record["record_offset"]
        record_size = image[record_offset]
        record_bytes = image[record_offset:record_offset + record_size]
        name_length = record_bytes[32]
        system_use_offset = 33 + name_length + (name_length % 2 == 0)
        system_use = record_bytes[system_use_offset:]
        if len(system_use) < 14 or system_use[6:8] != b"XA":
            raise RuntimeError(f"{path}: missing cdrtools XA System Use Information")
        image[record_offset + system_use_offset + 8] = number
        if path == "XA/FORM2.BIN":
            image[record_offset + system_use_offset + 4:record_offset + system_use_offset + 6] = (
                0x1011
            ).to_bytes(2, "big")

    dirs, extents = [], {}
    scan_tree(
        image,
        "",
        read32le(root_record, 2),
        read32le(root_record, 10),
        dirs,
        extents,
    )

    headers = [[0, 0, 0x08, 0] for _ in range(len(image) // BLOCK)]
    headers[16][2] = 0x09
    headers[17][2] = 0x89
    pvd_bytes = image[pvd:pvd + BLOCK]
    path_size = read32le(pvd_bytes, 132)
    path_starts = [read32le(pvd_bytes, 140), int.from_bytes(pvd_bytes[148:152], "big")]
    for start in path_starts:
        for index in range(start, start + math.ceil(path_size / BLOCK)):
            headers[index][2] = 0x89
    for directory in dirs:
        last = directory["extent"] + math.ceil(directory["length"] / BLOCK) - 1
        headers[last][2] = 0x89

    form2 = {}
    for path, (number, channel, submodes) in modes.items():
        record = extents[path]
        count = math.ceil(record["length"] / BLOCK)
        if count != len(submodes):
            raise RuntimeError(f"{path}: expected {len(submodes)} sectors, ISO has {count}")
        for local, submode in enumerate(submodes):
            sector = record["extent"] + local
            headers[sector] = [number, channel, submode, 0]
            if submode & 0x20:
                form2[sector] = form2_extra()

    stream = bytearray()
    for index, subheader in enumerate(headers):
        logical = image[index * BLOCK:(index + 1) * BLOCK]
        stream.extend(bytes(subheader))
        stream.extend(logical)
        stream.extend(form2.get(index, bytes(EXTRA)))
    return bytes(stream), headers, form2, extents, dirs, metadata_json(image, extents, dirs)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--vcdimager-source", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    source_tree = args.vcdimager_source
    output_dir = args.output_dir
    libvcd = source_tree / "lib/.libs/libvcd.a"
    if not libvcd.exists():
        raise SystemExit(f"build GNU VCDImager 2.0.1 first: missing {libvcd}")
    if '#define LIBVCD_VERSION "2.0.1"' not in (source_tree / "include/libvcd/version.h").read_text():
        raise SystemExit("the sector API source must be GNU VCDImager 2.0.1")
    mkisofs_version = subprocess.check_output(["mkisofs", "--version"], text=True)
    if "3.02a09" not in mkisofs_version:
        raise SystemExit("the fixture requires cdrtools mkisofs 3.02a09")
    output_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="mode-xa-fixture-") as temp:
        work = Path(temp)
        source = work / "src"
        source.mkdir()
        source_files = write_source_files(source)
        cooked_path = work / "mode2_xa_cooked.iso"
        env = dict(os.environ, TZ="UTC")
        subprocess.run([
            "mkisofs", "-quiet", "-no-pad", "-iso-level", "1", "-V", "MODEXA",
            "-sysid", "Ludared", "-A", "Synthetic ISO9660 XA fixture", "-xa",
            "-modification-date", "20261027000000+0000",
            "-o", str(cooked_path), str(source),
        ], check=True, env=env)
        cooked = bytearray(cooked_path.read_bytes())
        manifest, headers, form2, extents, dirs, metadata = make_manifest(cooked, source_files)
        manifest_path = work / "sectors.manifest"
        manifest_path.write_bytes(manifest)

        helper = Path(__file__).with_name("encode_mode2_sector.c")
        encoder = work / "encode_mode2"
        pkg_config_cflags = subprocess.check_output(
            ["pkg-config", "--cflags", "libcdio"], text=True
        ).split()
        pkg_config_libs = subprocess.check_output(
            ["pkg-config", "--libs", "libcdio"], text=True
        ).split()
        subprocess.run([
            "cc", f"-I{source_tree / 'include'}", *pkg_config_cflags,
            str(helper), str(libvcd), *pkg_config_libs, "-o", str(encoder),
        ], check=True)
        raw_path = output_dir / "mode2_xa_golden.bin"
        subprocess.run([str(encoder), str(manifest_path), str(raw_path)], check=True)
        (output_dir / "mode2_xa_cooked.iso").write_bytes(cooked)
        (output_dir / "mode2_xa_iso9660.json").write_bytes(metadata)
        print(f"raw sectors: {len(headers)}; Form 1: {len(headers) - len(form2)}; Form 2: {len(form2)}")
        print("files:")
        for path, record in sorted(extents.items(), key=lambda item: item[1]["extent"]):
            print(f"  {record['extent']:>3} {path} ({record['length']} bytes)")
        print(f"directories: {[(d['path'], d['extent'], d['length']) for d in dirs]}")


if __name__ == "__main__":
    main()
