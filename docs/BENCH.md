# The bench — checking OpenMicroCam on a YIZHAN 4K camera

How the YIZHAN 4K HDMI/USB microscope camera (SigmaStar SSC8836 / Mercury6, GC8613 sensor) is set up on a desk to check
`microcamd`: `cam check`, the testers under `tools/cam/test/`, the panel rig. The tools and their settings:
[TOOLS.md](TOOLS.md). Marks: [R] measured or read, [I] inference.

## Setting it up

- **Power and USB.** The camera on its own 12 V supply, its USB-C straight into a laptop port (no hub): the USB carries
  the camera's network (RNDIS), its webcam (UVC) and, on request, its SD card as a disk. After every camera reboot
  `tools/cam/cam-net.sh` gives the laptop its address on the new interface (`cam wait up` runs it).
- **An HDMI monitor** on the camera's HDMI output, on for the whole run: the HDMI checks time what reaches the display, and
  a monitor that switches inputs or sleeps changes the HDMI mode under a test. A 4K one shows every mode the camera offers.
- **An SD card** (FAT32) in the camera, with the camera's own files on it: photos `PHOTO/P00001.jpg` and
  `PHOTO/P00009.jpg` (take a few with the photo key) and at least one recording in `VIDEO/`. Recordings and photos made by
  the checks stay on the card.
- **The camera looking at the laptop's screen**, in focus, so that the latency clock (`tools/cam/panel/clock.py`, a
  fullscreen GTK 4 window on the laptop panel) fills its view: the JPEG-ring check needs a busy picture, the panel rig
  reads the clock (calibrate its view first: `tools/cam/panel/view.py cal`). Whatever the camera sees serves every other check.
- **A USB viewer, for the full set.** The checks of a paused output, the player, the card and recording run beside a host
  that streams the camera over USB, as in use: keep a viewer open (`tools/cam/usb-preview.sh`, or your own) and set the
  stream hooks ([TOOLS.md](TOOLS.md) «Hooks») so the checks that open the video node themselves can free it and give it
  back. Without the hooks a viewer holding the node fails those checks.
- **On the laptop:** Python 3 with numpy, ffmpeg and ffprobe, `v4l2-ctl` (v4l-utils), GTK 4 with PyGObject (the clock),
  `udisksctl` (the card check), and passwordless `sudo` for `ip` (`cam-net.sh`) and for `modprobe usbmon` / reading
  usbmon (the stream-answer check).
- **The settings:** the camera's login when it is not the stock one (`MICROCAM_PASSWORD`) and, for `cam deploy`, the build
  inputs — [TOOLS.md](TOOLS.md) «Settings».

## Running

`cam state` first: it names what is wrong now (a stray holder of the video node, failing stream starts, a rebuild loop, a
reset since the last look, a sensor mode or USB format list microcamd could not use and what it runs instead). Then `cam check` (the smoke set) or `cam check full`; one line a case, PASS / FAIL by its
thresholds or `----` when only measured, the details in `$MICROCAM_LOGS/check-<date>.log` and the testers' own logs; the
exit code is the number of failures. Run one check at a time: two share the stream, the menu and the recording and fail
each other. The smoke set takes about 30 s [R]; the full set about six minutes [R one run].

| check (tester) | what it does | needs besides the camera on USB |
|---|---|---|
| smoke: menu memory | the demo menu shown and left: the menu's memory back to 0 | — |
| smoke: no pool stuck | no canvas pool closing for over a second | — |
| smoke: stream start and stop | MJPEG 4K30 from the laptop: ≥ 55 of 60 frames, the stop seen within 5 s | the video node free (hooks) |
| smoke: `uvcanswer.py` | a start no rule serves is answered, the next normal one works | usbmon (sudo) |
| full: `heavy.py` | heavy menu layouts: no refused commit, nothing not shown, the menu's memory back | monitor; the two card photos |
| full: `cursor.py` | a cursor moved by commits, paced by HDMI: median commit → shown ≤ 40 ms | monitor |
| full: `onepool.py` | a 4K recording sharing HDMI's frame, the USB stream stopped and started under it; a photo during it | card; viewer with hooks |
| full: `rings.py` | USB's JPEG rings at 4K30, 4K45, 1080p120: rate, drops, oversize | the camera looking at the clock on the laptop panel |
| full: `pause.py` | one output paused beside a recording, the other's rate kept | monitor; card; viewer |
| full: `play.py` | the player: play, pause, seek, the end, the menu over it | card with a recording; monitor; viewer |
| full: `card.py` | the card as the laptop's disk and back to the camera | card; `udisksctl`; viewer |
| full: `record.py` | a 4K recording in H.265 and H.264, pulled and decoded | card; ffmpeg; viewer |

## Uploads to a stock camera

With the stock USB controller driver a full-speed upload over the camera's USB network hangs or panics the camera [R]:
set the MTU to 400 on both ends (`ifconfig usb0 mtu 400` on the camera, `ip link set <if> mtu 400` on the laptop, again
after every reboot) or serve the files at a low rate with `tools/cam/serve-slow.py`, and upload with no USB video
running. `microcamd`'s own USB controller driver takes full-speed uploads.

## Serial console

The console, its adapter and `tools/cam/uart.py`: [SHELL-ACCESS.md](SHELL-ACCESS.md).
