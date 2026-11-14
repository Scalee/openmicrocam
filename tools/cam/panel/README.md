# The panel rig: tests that show the camera a picture on the laptop panel
First the camera's view is calibrated (`view.py cal`, a second or two): which piece of the panel it sees, turned and at
a slant, as a homography; every test picture is then drawn pre-distorted through it, upright and filling the camera's
frame, with a thin green outline of the view just inside the frame's edge. The pictures: the latency clock and the
moving picture for dropped frames (`clock.py`).

## View calibration
- `view.py cal --size 1920x1080` shows tiles coding their own place on the panel, grabs the USB stream until three are
  read (the camera's exposure settles in a frame or two), fits the homography and saves it per size in
  `$MICROCAM_LOGS/panel/view.json` (this rig's files — `view.json`, `panel.json`, `results.jsonl` — live there); the
  outline then stays on the panel for 3 s. «no tile read»: the aim, the focus, or another `--cell`.
- Calibrate each size whose field of view differs (crop modes); a size not calibrated takes another's, scaled.

## Dropped frames from a recording
- `clock.py --motion --size 1920x1080` (a bar sweeping between two rows of clock tiles); record; then
  `framegaps.py FILE|URL` prints the frames read, the frames lost (a step between frames longer than the file's
  period) and repeated (no step) — from the picture, so it sees what the MP4's timestamps cannot.

## Glass-to-glass latency (USB: MJPEG, H.264, H.265)
The laptop panel shows `clock.py` (tiles of 4×8 cells: a Gray-coded counter of display frames, 8.33 ms each, the tile's
height on the panel, reference cells); the camera looks at the panel; `latency.py` grabs the USB stream and reads the
clock in one tile per band of rows of every frame. A reading = the frame's arrival on the laptop (its first packet,
`v4l2-ctl`'s CLOCK_MONOTONIC stamp moved to epoch time; with `--end` its last, ffmpeg's wall clock at the read) − the
middle of that row's exposure. The readings fall along the frame's height by the sensor's readout; their slope is
printed as `readout` and the latency is given for the frame's first row. It includes the panel's own response (a
constant), the sensor and the whole camera path. H.264 / H.265 streams are decoded whole by ffmpeg (`--codec h264|h265`).

## How a reading is made exact (2026-11-11)
- Exposure: for the grab the rig sets the camera's exposure to one clock step (8.3 ms, gain ×4 by `--gain`) and puts the
  camera's own back after. With the camera's auto exposure (~25 ms [I, from the contrast]) a cell blended up to four
  clock values: 10–25 % of frames unread, p10–p90 18–28 ms; with a short one all read, 7–11 ms [R 4K30 MJPEG, 4 s runs, 3 and 4 runs].
- Inside the step: during a one-step exposure exactly one cell changes; its level is the share of the exposure after
  the change, so the time is read inside the step instead of rounded to it. A camera rate near a divisor of the
  panel's 120 Hz (30) otherwise keeps each tile in one phase and the rounding never averages out (the readout came out
  18 ms at 4K30 against 21 at 4K45 of the same sensor mode).
- The panel's table (`panel.json`, `latency.py panel`, made by `matrix` when missing): a cell does not change in an
  instant — at half a step a lighting cell reads 0.69, a darkening one 0.34 [R 2026-11-11]. At 45 fps the exposures fall
  at every phase of the refresh, so the measured shares' quantiles map back to even true shares. Made again for another
  panel or brightness.
- The panel lights its rows top to bottom over a refresh: each tile carries its height (5 cells), and its reading is
  moved by that share of a step.
- Checked [R 2026-11-11]: the readout equals the sensor's 2160 rows × 9.48 µs =
  20.48 ms — 4K45 20.5 / 20.6, 4K30 19.3 / 21.2 (one run each, the panel's table in), every mode of the matrix 20.5–21.4;
  the four tiles lie on the line within 0.4 ms.

## The clock on the camera's screen
- `clock.py` (GTK 4) opens fullscreen on the laptop panel (the monitor whose connector is `eDP*`, `--monitor` for
  another) by itself and quits on Esc / q or after `--secs`; it draws in the view of `--size` (`--whole`: over the whole
  panel at `--cell` px). It numbers each frame for the time the compositor predicts it reaches the panel. Compare runs of
  one clock and one rig only. `latency.py` reads the tiles where the clock draws them.

## Measuring
- `latency.py matrix --modes 3840x2160@30:mjpeg,3840x2160@30:h264,… [--runs N] [--secs S] [--end] [--tag T]` calibrates
  each size's view once, shows the clock itself, makes the panel's table when missing, frees the
  USB stream through the host's stream hooks (`docs/TOOLS.md`), runs every mode (2 s each by default) and prints one table (the median of each run's
  median, min and max over runs, the readout, decoded frames).
- One run by hand: `clock.py --size … &`, then `latency.py run --size … --fps … --codec … --tag TEXT [--frames F]` (appends to
  `results.jsonl`; `--frames` writes each frame's readings per tile). The panel is 120 Hz: `--p 8.333`, the same in `clock.py --p`.
- A wallpaper change is the static case (`tools/cam/screen.sh`, run with `sh`).

## Inside the camera: how long each reader holds a canvas
- `holds.py [--json] [--raw FILE]` reads `/proc/microcam_canvas_holds` (the module keeps each reader's last 256 releases:
  canvas, hand-over and release in ktime ns) and prints per reader the hold in ms and the hold series. DISP lets a canvas
  go at a vsync, so its hold is the display's phase against the hand-over; read it a few times to see it drift.
