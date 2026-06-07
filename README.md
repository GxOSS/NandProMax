# NandProMax

NAND and CPLD Reader / Writer for x86/ARM64 Windows, Mac and Linux

Inspired by NANDPro, FTDI2SPI, and XSVFTool. Aims to do everything NANDPro could do, with FTDI and PicoFlasher support, and a much cleaner UX.

Supports inputting a Genexis / gxBuild LBAMap for On-The-Fly bad block remapping, and will prompt user with options if unaccounted bad blocks are found.

## Operations

- SPI Read / Write
- eMMC Read / Write
- Remap Bad Blocks
- Detect CPLD and play XSVF (and SVF)


## Devices

If anybody wants to work in PICFLASH (MTX, JRP, NAND-X) support, be my guest. Same goes for the TX DemoN. Otherwise, just use a pico or xflasher.

DirtyJTAG support was planned but CMSIS-DAP makes much more sense. Can be used on more then just a pico

- XFlasher360
- PicoFlasher v4+
- CMSIS
- ~~"DirtyPico" (DirtyJTAG)~~
- ESPFlasher
