# Installing OpenMicroCam on the YIZHAN 4K microscope camera

For the YIZHAN 4K HDMI/USB microscope camera (SigmaStar SSC8836 / Mercury6, GC8613); the tested board and firmware are in
[HARDWARE.md](HARDWARE.md).

You need Linux (Debian/Ubuntu); Windows and macOS are not tried. The camera is connected to the computer over USB. Run
every command from the repository's root; the files they make are kept in `work/`.

## Save the original firmware

The dump of the camera's flash is what the build takes from the camera and the way back to the original firmware.

Install what unpacks the dump:

```sh
sudo apt install squashfs-tools pipx && pipx install jefferson
```

Take the microSD card out of the camera, if one is in it, and power the camera on again: with a card in, the computer
sees the camera only as a USB drive, without the network the installer works over.

Take the dump: the installer reads the flash over the camera's USB network and checks that it carries the firmware
OpenMicroCam is tested with.

```sh
tools/install/install.py dump
```

After a successful run the dump is in `work/dump`: `nor.bin` is the camera's flash, the copy of the original firmware;
the rest is the same flash unpacked, which the build reads.

## Build OpenMicroCam

The build sets up the tools and a kernel tree like the camera's, then builds microcamd, the kernel modules and the
USB gadget against the dump's libraries.

```sh
tools/install-toolchain.sh                             # Rust, zig, the arm cross compiler
sudo apt install make gcc flex bison bc
export PATH=$HOME/.cargo/bin:$HOME/.local/bin:$PATH
tools/mk-linklibs.sh                                   # the dump's libraries for linking
tools/install/prepare-ktree.sh                         # Linux 4.9.227 set up like the camera's kernel
cargo zigbuild --release --target armv7-unknown-linux-gnueabihf.2.30                                           # microcamd
for m in microcam-sys microcam-canvas microcam-rgn gc8613 udc-msb250x; do sh kmods/build-ko.sh kmods/$m; done  # kernel modules
kmods/g_webcam/build.sh                                # the USB gadget
camera/tools/build.sh camera/tools/fbhide.c
```

## Install or update OpenMicroCam

The installer uploads the build to the camera, installs it and reboots the camera. An update is the same command after
a new build.

```sh
tools/install/install.py install
```

## Back to the original firmware

The installer puts the original files and settings from the dump back on the camera; with `-n` it only shows what it
would change.

```sh
tools/install/install.py uninstall
```

## Other ways

### With a microSD card

Without the USB network: the camera takes the dump, installs and goes back by itself, when powered on with the card. The
card is FAT32.

The dump. The installer prepares the card's files in `work/card/dump`; copy them into the card's root and power the
camera on with it — the camera writes its flash into the file `FLASH.BIN` on the card.

```sh
tools/install/install.py dump --card
```

Put the card's `FLASH.BIN` back into `work/card/dump`; the same command takes the dump from it and checks it.

```sh
tools/install/install.py dump --card
```

The install. The installer prepares the files in `work/card/install`; copy them into the card's root and power the camera
on with it — it installs OpenMicroCam and reboots. If it did not, the reason is in `OPENMICROCAM/install.log` on the
card.

```sh
tools/install/install.py install --card
```

Back to the original firmware — the same way, the files in `work/card/stock`.

```sh
tools/install/install.py uninstall --card
```

### Over the serial console (UART)

The serial console brings the factory login back, takes the full dump and restores a region of the flash. How to connect
to the UART: [SHELL-ACCESS.md](SHELL-ACCESS.md). Keep the bridge to the console running in a separate terminal.

```sh
python3 tools/cam/uart.py bridge
```

The factory login back:

```sh
tools/install/install.py access
```

The full dump: a microSD card of 1 GB or more whose contents are overwritten.

```sh
tools/install/install.py dump --serial                     # the card in the camera
tools/install/install.py dump --serial --reader /dev/sdX   # then the card in the card reader
```

A region of the flash back from the dump, through a microSD card:

```sh
tools/install/restore-region.sh REGION /dev/sdX            # the card in the card reader
tools/install/restore-region.sh REGION                     # then the card in the camera
```
