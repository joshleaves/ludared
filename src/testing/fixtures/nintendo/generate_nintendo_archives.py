#!/usr/bin/env python3
"""Regenerate the Nintendo DARC/SARC and LZ10/LZ11 test fixtures.

Every fixture is produced by a third-party implementation, never by Ludared:

- ``nintendo_darc.arc``   dnasdw/darctool 1.2.0 (``-cvfd``, 32-byte alignment)
- ``nintendo_sarc.sarc``  Tyulis/3DSkit (``-p -f SARC -l -d``, 128-byte padding)
- ``nintendo_*.lz10``     magical/nlzss ``compress.py``
- ``nintendo_*.lz11``     magical/nlzss ``compress_nlz11``

The tools are not vendored. Check them out and build darctool first::

    git clone https://github.com/dnasdw/darctool
    git -C darctool submodule update --init --recursive
    cmake -S darctool -B darctool/build -DCMAKE_BUILD_TYPE=Release \\
        -DCMAKE_POLICY_VERSION_MINIMUM=3.5
    cmake --build darctool/build -j
    git clone https://github.com/Tyulis/3DSkit
    git clone https://github.com/magical/nlzss

Then, from the repository root::

    python3 src/testing/fixtures/nintendo/generate_nintendo_archives.py --tools .fixture-tools
    python3 src/testing/fixtures/nintendo/generate_nintendo_archives.py --tools .fixture-tools --check
    python3 src/testing/fixtures/nintendo/generate_nintendo_archives.py --tools .fixture-tools --verify-interop

``--check`` rebuilds every fixture in memory and compares it with the committed
file instead of writing it. The script also re-extracts every archive with the
tool that produced it and decompresses every stream with nlzss, so a broken or
unexpected fixture fails here rather than in the test suite.

``--verify-interop`` checks the other direction: it decodes each fixture with
Ludared, rebuilds it unchanged, decompresses streams with nlzss, extracts the
archives with the third-party tools and compares every path and file with the
source tree. It then repeats that with ``hi.txt`` replaced through Ludared's
override workflow. It needs a built Ludared binary (``--ludared``, default
``target/debug/ludared``).
"""

import argparse
import hashlib
import io
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

FIXTURE_DIR = Path(__file__).resolve().parent
REPO_ROOT = FIXTURE_DIR.parents[3]
SEED = 0x4C55_4441  # "LUDA"

DARC_CODEC = "std/nintendo/container/darc"
SARC_CODEC = "std/nintendo/container/sarc"
LZ10_CODEC = "std/nintendo/compression/lz10"
LZ11_CODEC = "std/nintendo/compression/lz11"

# Every fixture, the codec that decodes it, the archive it wraps (None for an
# uncompressed archive), and the container format of that archive.
FIXTURES = {
    "nintendo_darc.arc": (DARC_CODEC, None, None, "darc"),
    "nintendo_sarc.sarc": (SARC_CODEC, None, None, "sarc"),
    "nintendo_darc.lz10": (LZ10_CODEC, {"container": "darc"}, "nintendo_darc.arc", "darc"),
    "nintendo_darc.lz11": (LZ11_CODEC, {"container": "darc"}, "nintendo_darc.arc", "darc"),
    "nintendo_sarc.lz10": (LZ10_CODEC, {"container": "sarc"}, "nintendo_sarc.sarc", "sarc"),
    "nintendo_sarc.lz11": (LZ11_CODEC, {"container": "sarc"}, "nintendo_sarc.sarc", "sarc"),
}


def xorshift32(length, seed=SEED):
    x = seed
    data = bytearray()
    while len(data) < length:
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        data.append(x & 0xFF)
    return bytes(data[:length])


CONTENT = {
    "hi.txt": b"hello, darc & sarc!\n",
    "data/pattern.bin": b"\x01\x02\x03\x04\x05\x06\x07\x08" * 160,
    "data/nested/noise.bin": xorshift32(777),
}

# Deterministic replacement contents for the modified-archive checks.
REPLACEMENT = {"hi.txt": xorshift32(101, 0x5245_504C)}  # "REPL"


def write_content(root):
    for name, data in CONTENT.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def run(command, **kwargs):
    result = subprocess.run(command, capture_output=True, text=True, **kwargs)
    if result.returncode != 0:
        raise SystemExit(
            f"command failed: {' '.join(map(str, command))}\n{result.stdout}\n{result.stderr}"
        )
    return result


# --- fixture generation ----------------------------------------------------


def build_darc(darctool, content, work):
    archive = work / "nintendo_darc.arc"
    run([str(darctool), "-cvfd", str(archive), str(content), "-a", "32"])

    extracted = work / "extracted-darc"
    run([str(darctool), "-xvfd", str(archive), str(extracted)])
    for name, data in CONTENT.items():
        if (extracted / name).read_bytes() != data:
            raise SystemExit(f"darctool re-extracted {name} differently")
    return archive.read_bytes()


def build_sarc(kit, content, work):
    archive = work / "nintendo_sarc.sarc"
    run(
        [
            sys.executable,
            str(kit),
            "-p",
            "-f",
            "SARC",
            "-l",
            "-d",
            "-o",
            str(archive),
            str(content),
        ],
        cwd=str(kit.parent),
    )

    run([sys.executable, str(kit), "-x", "-f", "SARC", str(archive)], cwd=str(kit.parent))
    extracted = archive.with_suffix("")
    for name, data in CONTENT.items():
        matches = [
            path
            for path in extracted.rglob("*")
            if path.is_file() and path.as_posix().endswith(name)
        ]
        if not matches or matches[0].read_bytes() != data:
            raise SystemExit(f"3DSkit re-extracted {name} differently")
    return archive.read_bytes()


def compress(nlzss, data, kind):
    sys.path.insert(0, str(nlzss))
    import compress as nlzss_compress
    import lzss3

    stream = io.BytesIO()
    if kind == 10:
        nlzss_compress.compress(data, stream)
    else:
        nlzss_compress.compress_nlz11(data, stream)
    compressed = stream.getvalue()
    if lzss3.decompress_bytes(compressed) != data:
        raise SystemExit(f"the LZ{kind} stream does not decompress to its input")
    return compressed


def build_fixtures(tools, work):
    darctool = tools / "dnasdw_darctool" / "bin" / "Release" / "darctool"
    kit = tools / "3DSkit" / "3DSkit.py"
    nlzss = tools / "nlzss"
    for tool in (darctool, kit, nlzss / "compress.py", nlzss / "lzss3.py"):
        if not tool.exists():
            raise SystemExit(f"missing {tool}; see the module docstring")

    content = work / "content"
    write_content(content)

    fixtures = {}
    fixtures["nintendo_darc.arc"] = build_darc(darctool, content, work)
    fixtures["nintendo_sarc.sarc"] = build_sarc(kit, content, work)

    archives = {
        "darc": fixtures["nintendo_darc.arc"],
        "sarc": fixtures["nintendo_sarc.sarc"],
    }
    for kind in (10, 11):
        for label, archive in archives.items():
            fixtures[f"nintendo_{label}.lz{kind}"] = compress(nlzss, archive, kind)
    return fixtures


# --- reverse-direction interoperability ------------------------------------


def nlzss_decompress(nlzss, data):
    sys.path.insert(0, str(nlzss))
    import lzss3

    return lzss3.decompress_bytes(data)


def extract_darc(darctool, archive, work):
    path = work / "rebuilt.arc"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(archive)
    out = work / "extract"
    run([str(darctool), "-xvfd", str(path), str(out)])
    return {
        item.relative_to(out).as_posix(): item.read_bytes()
        for item in out.rglob("*")
        if item.is_file()
    }


def extract_sarc(kit, archive, work):
    path = work / "rebuilt.sarc"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(archive)
    run([sys.executable, str(kit), "-x", "-f", "SARC", str(path)], cwd=str(kit.parent))
    out = path.with_suffix("")
    return {
        item.relative_to(out).as_posix(): item.read_bytes()
        for item in out.rglob("*")
        if item.is_file()
    }


class Project:
    """A throwaway Ludared project driven through the real binary."""

    CONFIG = """\
[project]
name = "nintendo-interop"
manifest = "interop.ludared"

[paths]
sources = "sources"
workspace = "workspace"
scripts = "scripts"
builds = "builds"
cache = "builds/cache"
"""

    def __init__(self, binary, root):
        self.binary = binary
        self.root = root
        (root / "sources").mkdir(parents=True)
        (root / "ludared.toml").write_text(self.CONFIG)
        (root / "interop.ludared").write_text('{ "sources": {} }')

    def ok(self, *args):
        return run([str(self.binary), *args], cwd=str(self.root)).stdout

    def add_source(self, name, data):
        (self.root / "sources" / name).write_bytes(data)
        self.ok("sources", "add", name)

    def decode(self, name, codec, args):
        self.ok("decode", "add", name, codec, json.dumps(args or {}))

    def unpack(self):
        self.ok("unpack")

    def build(self, name):
        self.ok("build", name)
        return (self.root / "builds" / name).read_bytes()

    def override(self, virtual_path, data):
        self.ok("override", "add", virtual_path)
        path = self.root / "workspace" / virtual_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def ludared_rebuild(binary, name, data, codec, args, work, edits=None):
    project = Project(binary, work)
    project.add_source(name, data)
    project.decode(name, codec, args)
    project.unpack()
    for virtual_path, payload in (edits or {}).items():
        project.override(f"{name}/files/{virtual_path}", payload)
    return project.build(name)


def verify_interop(tools, binary, work):
    darctool = tools / "dnasdw_darctool" / "bin" / "Release" / "darctool"
    kit = tools / "3DSkit" / "3DSkit.py"
    nlzss = tools / "nlzss"
    if not binary.is_file():
        raise SystemExit(f"missing {binary}; run `cargo build` first")
    extractors = {"darc": (darctool, extract_darc), "sarc": (kit, extract_sarc)}

    committed = {name: (FIXTURE_DIR / name).read_bytes() for name in FIXTURES}
    archives = {
        "nintendo_darc.arc": committed["nintendo_darc.arc"],
        "nintendo_sarc.sarc": committed["nintendo_sarc.sarc"],
    }
    modified = dict(CONTENT)
    modified.update(REPLACEMENT)

    failures = []
    stream_notes = []

    def check(label, ok, detail=""):
        print(f"{'ok  ' if ok else 'FAIL'}  {label}{'' if ok else '   ' + detail}")
        if not ok:
            failures.append(label)

    for name, (codec, args, wrapped, container) in FIXTURES.items():
        tool, extract = extractors[container]
        rebuild = lambda edits=None, tag="unmodified": ludared_rebuild(  # noqa: E731
            binary, name, committed[name], codec, args, work / f"{tag}-{name}", edits
        )

        rebuilt = rebuild()
        if wrapped is None:
            check(f"{name}: bit-exact rebuild", rebuilt == committed[name])
            archive = rebuilt
        else:
            archive = nlzss_decompress(nlzss, rebuilt)
            check(
                f"{name}: nlzss accepts Ludared's stream",
                archive == archives[wrapped],
                f"{len(archive)} vs {len(archives[wrapped])} bytes",
            )
            stream_notes.append((name, rebuilt == committed[name], len(rebuilt), len(committed[name])))

        extracted = extract(tool, archive, work / f"extract-{name}")
        check(f"{name}: third-party extraction matches the source tree", extracted == CONTENT)

        rebuilt = rebuild(REPLACEMENT, "modified")
        archive = nlzss_decompress(nlzss, rebuilt) if wrapped else rebuilt
        extracted = extract(tool, archive, work / f"extract-modified-{name}")
        check(f"{name}: modified archive extraction matches", extracted == modified)

    print("\nLudared stream vs nlzss output (informational):")
    for name, identical, size, fixture_size in stream_notes:
        state = "byte-identical" if identical else "different encoding"
        print(f"  {name:22} {state:18} {size:5} bytes (fixture: {fixture_size})")

    if failures:
        raise SystemExit(f"{len(failures)} interoperability check(s) failed: {', '.join(failures)}")


def main():
    parser = argparse.ArgumentParser(description="Regenerate the Nintendo test fixtures.")
    parser.add_argument(
        "--tools",
        type=Path,
        default=Path("."),
        help="directory holding the third-party checkouts (default: the current directory)",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check",
        action="store_true",
        help="compare with the committed fixtures instead of writing them",
    )
    mode.add_argument(
        "--verify-interop",
        action="store_true",
        help="rebuild every fixture with Ludared and re-extract it with the third-party tools",
    )
    parser.add_argument(
        "--ludared",
        type=Path,
        default=REPO_ROOT / "target" / "debug" / "ludared",
        help="Ludared binary to drive (default: target/debug/ludared in this repository)",
    )
    args = parser.parse_args()

    if args.verify_interop:
        work = Path(tempfile.mkdtemp(prefix="nintendo-interop-"))
        try:
            verify_interop(args.tools.resolve(), args.ludared.resolve(), work)
        finally:
            shutil.rmtree(work, ignore_errors=True)
        return

    work = Path(tempfile.mkdtemp(prefix="nintendo-fixtures-"))
    try:
        fixtures = build_fixtures(args.tools.resolve(), work)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    failed = False
    for name, data in fixtures.items():
        path = FIXTURE_DIR / name
        digest = hashlib.sha256(data).hexdigest()
        if not args.check:
            path.write_bytes(data)
            print(f"wrote    {name:22} {len(data):5} bytes  {digest}")
        elif path.exists() and path.read_bytes() == data:
            print(f"ok       {name:22} {len(data):5} bytes  {digest}")
        else:
            failed = True
            print(f"MISMATCH {name:22} expected {len(data):5} bytes  {digest}")
    if failed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
