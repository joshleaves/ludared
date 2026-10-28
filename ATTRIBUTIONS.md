# Attributions and licenses

There's no way I could've ever built ludared without the HUGE AND WONDERFUL work of the giants[^1] whose shoulders I stand on. A lot of codec implementations comes from their work, when it's not directly translated in rust to be integrated to ludared.

# Codec: `std/nintendo/snes/cart/*`

Everything I learned about SNES ROMs comes from:
- [fullsnes by nocash](https://problemkaputt.de/fullsnes.htm), whichs is really the bible of the console.
- [The SNESdev wiki](https://snes.nesdev.org/wiki/SNESdev_Wiki), which was a tad easier to  read.
- [SuperFamiCheck](https://github.com/Optiroc/SuperFamicheck) is the definitive code for SNES checksum header calculations (SPC7110 is a bitch).

# Codec: `std/nintendo/3ds/container/cci`

The codec implementation was based on the following references:
- [ctrtool](https://github.com/3DSGuy/Project_CTR)
- [pyctr](https://github.com/ihaveamac/pyctr)

Those two are the definitive implementations.

# Codecs: `std/nintendo/3ds/container/cia`, `std/nintendo/3ds/internal/*`

The format layouts come from [3dbrew](https://www.3dbrew.org/wiki/Main_Page),
the community wiki that documents the 3DS better than any manual ever will.

The RomFS rebuild (`std/nintendo/3ds/internal/romfs`) is a rust port of the
tree layout, hash-table and IVFC hashing algorithms from
[makerom](https://github.com/3DSGuy/Project_CTR)'s `romfs_gen.c`, so images
rebuilt by ludared are laid out the way Nintendo's own tooling lays them out.

makerom is distributed under the MIT license, whose notice is reproduced here
as required for the ported portions:

> MIT License
>
> Copyright (c) 2014 3DSGuy
> Copyright (c) 2014 applestash
> Copyright (c) 2015-2022 Jakcron
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in
> all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

# Codec: `std/nintendo/font/bcfnt`

The codec is a Rust port of the BCFNT format handling and texture algorithms in [ObsidianX/3dstools `bcfnt.py`](https://github.com/ObsidianX/3dstools/blob/master/bcfnt.py). The repository does not state a license, so the port is attributed here without asserting a license for the upstream work.

# Codec: `std/sega/compression/prs`

As their GH description claims, indeed, the fastest PRS implementation in rust belongs to [Sewer56's prs-rs](https://github.com/Sewer56/prs-rs).

While the codec's approach is different and independent (and in safe-Rust!), some tricks (notably contiguous storage of positions) were inspired by their algorithm. 

Their crate is also included in the dev-dependencies to produce test fixtures, and double-check that our encodes can be decoded by another PRS decompressor.

***
[^1]: Whom I hope I can one day call "peers".
