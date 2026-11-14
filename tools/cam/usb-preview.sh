#!/bin/bash
# Low-latency USB preview of the microscope for latency measurement.
# usage: usb-preview.sh [mjpeg|h264] [1920x1080|3840x2160] [fps, default 30]
FMT=${1:-mjpeg}; SIZE=${2:-1920x1080}; FPS=${3:-30}
DEV=$(eval "$(python3 "$(dirname "$0")/../hostconf.py" --sh)" && echo "$MICROCAM_VIDEO")
exec ffplay -hide_banner -loglevel warning \
  -f v4l2 -input_format "$FMT" -video_size "$SIZE" -framerate "$FPS" \
  -fflags nobuffer -flags low_delay -framedrop -probesize 32 -analyzeduration 0 -sync ext \
  -fs -window_title "Microscope USB $FMT $SIZE low-latency" "$DEV"
