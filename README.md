# NandProMax

NAND and CPLD Reader / Writer for x86/ARM64 Windows, Mac and Linux

Inspired by NANDPro, FTDI2SPI, and XSVFTool. Aims to do everything NANDPro could do, with FTDI and PicoFlasher support, and a much cleaner UX.

Planned support for active bad block remapping

## Operations

- SPI Read / Write
- WIP eMMC Read / Write
- WIP Remap Bad Blocks
- WIP Detect CPLD and play XSVF (and SVF?)


## Devices

If anyone has a JRP, NAND-X, Matrix SPI, Maximus SPI, or TX Demon, please go and test [tuxuser's fork](https://github.com/tuxuser/NandProMax)

If anyone has a 360Squirt please test this repo

DirtyJTAG support was planned but CMSIS-DAP makes much more sense. Can be used on more then just a pico

- XFlasher360 / FT2232
- PicoFlasher v4+ / RP2040
- CMSIS-DAP / RP2040 and more
- ESPFlasher TCP / ESP32

WIP
- TX Demon
- Pico W TCP and USB
- Pico RP2350 / W TCP and USB
- 360squirt Squirt Programmer
- NAND-X
- JR-Programmer v1/v2
- Matrix SPI
- More microchips?


## Build

```
cargo build --release
```
