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

***
[^1]: Whom I hope I can one day call "peers".