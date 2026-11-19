# OpenMicroCam — open firmware for the YIZHAN 4K microscope camera (XHU4012)

![OpenMicroCam — open firmware for the YIZHAN 4K (XHU4012) HDMI/USB microscope camera, SigmaStar SSC8836, GC8613: the camera and its picture with the on-screen menu](docs/images/yizhan-4k-microscope-camera-openmicrocam.png)

OpenMicroCam is an independent open firmware for the **YIZHAN 4K** HDMI/USB microscope camera, built on the SigmaStar
SSC8836 (Mercury6) with a GalaxyCore GC8613 sensor. It began with a sensor driver of its own — the stock one has a single
mode — and grew into a daemon, `microcamd`, that takes the place of the vendor's application: live video to HDMI and USB
(UVC), an on-screen menu, photos and recording.

> **Status:** everything described here works on the tested camera. The documentation, the install scripts and the tools
> are still being tidied up.

## How it differs from the stock firmware

The stock firmware captures only 4K at 30 frames a second: its sensor driver has no other mode. The 60 frames of 1080p on
HDMI it advertises are each frame repeated. OpenMicroCam has a sensor driver of its own with a mode for each output, and
every frame in a second is a real one.

| | Stock | OpenMicroCam |
|---|---|---|
| USB | 4K, 2K, 1080p — up to 30 fps | 4K up to 45, 2K up to 66, 1080p up to 120 fps |
| HDMI | 4K30; 1080p60 from 30 real frames | 4K30, 2K60, 1080p60 |
| Recording | 4K30 and 1080p30, 10 Mbit/s | 4K up to 25, 2K up to 50, 1080p up to 100 fps; 4K at 20 Mbit/s |

- **Low latency over USB:** in H.264 4K a frame reaches the computer in about 40 ms.
- **The monitor is not reset:** the stream to the computer starts and stops without interrupting the picture on the monitor.
- **Recording does not stop the rest:** it runs beside the stream and the monitor, and a photo can be taken while recording.
- **Sharper in motion:** the noise reduction that smears texture in motion on the stock firmware is off.
- **The menu in the USB video:** seen on the monitor and on the computer alike.

## Supported hardware

| | |
|---|---|
| Tested camera | YIZHAN 4K, model XHU4012 — sold as «YIZHAN 4K 60MP UHD … HDMI Type-C» digital microscope camera; the housing reads «YIZHAN 4K ULTRA HD» |
| Board | marked «oseecam WS03 V5.0 2023.09.16» |
| SoC | SigmaStar SSC8836 (Mercury6) |
| Sensor | GalaxyCore GC8613, 1/2.7" — sellers' listings name it «IMX415» or «IMX415L»; the 1/2.7" size in a listing gives it away (Sony's IMX415 is 1/2.8") |
| Flash | 16 MB SPI NOR |

Other 4K microscope cameras on the same SigmaStar platform are probable candidates, none of them tested. A camera sold
under another name may carry a different board, sensor or stock firmware version. The exact marks and the candidates:
[docs/HARDWARE.md](docs/HARDWARE.md).

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — how `microcamd` is built: its blocks, the pipeline model, the data paths.
- [docs/HARDWARE.md](docs/HARDWARE.md) — the tested camera with its exact marks, and untested candidates.
- [docs/INSTALL.md](docs/INSTALL.md) — dump your camera, check it, build, install, and back to stock, all over its USB
  cable; the stock camera's shell login is its known default, `root` / `ws123321`.
- [docs/firmware/INDEX.md](docs/firmware/INDEX.md) — the specification of the camera's stock firmware, component by
  component: boot chain, kernel, SigmaStar MI and mhal media stack, ISP, codecs, the vendor's applications.

## How to install

OpenMicroCam runs on the stock firmware's kernel and video drivers for now. They are not in this repository: the build
takes them from a dump of your own camera, so the dump comes first.

You need a computer with Linux (Debian/Ubuntu), the camera connected to it over USB. The commands run in order from the
repository's root; everything they make goes into `work/`.

```sh
git clone https://github.com/apnix/openmicrocam && cd openmicrocam

# save the camera's original firmware: the build takes from it, and it is the way back
sudo apt install squashfs-tools pipx && pipx install jefferson
tools/install/install.py dump                          # ends with «OK»

# build
tools/install-toolchain.sh                             # Rust, zig, the arm cross compiler
sudo apt install make gcc flex bison bc
export PATH=$HOME/.cargo/bin:$HOME/.local/bin:$PATH
tools/mk-linklibs.sh                                   # the dump's libraries for linking
tools/install/prepare-ktree.sh                         # Linux 4.9.227 set up like the camera's kernel
cargo zigbuild --release --target armv7-unknown-linux-gnueabihf.2.30
for m in microcam-sys microcam-canvas microcam-rgn gc8613 udc-msb250x; do sh kmods/build-ko.sh kmods/$m; done
kmods/g_webcam/build.sh                                # the USB gadget
camera/tools/build.sh camera/tools/fbhide.c

# install; an update is the same command after a new build
tools/install/install.py install                       # the camera reboots into OpenMicroCam
```

Back to the original firmware:

```sh
tools/install/install.py uninstall
```

Installing from a microSD card (when the camera does not come up on USB) and over the serial console:
[docs/INSTALL.md](docs/INSTALL.md).
