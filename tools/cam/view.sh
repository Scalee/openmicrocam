#!/bin/sh
# Low-latency viewers of the microscope's USB stream, fullscreen on the laptop panel (eDP-1, 120 Hz) — for latency comparison.
# usage: view.sh mpv|wl|gl [WxH=3840x2160] [fps=40] [screen=eDP-1|HDMI-1]      quit: q / Esc (mpv), close window (gst)
V=${1:-mpv}; SIZE=${2:-3840x2160}; FPS=${3:-40}; SCR=${4:-eDP-1}; W=${SIZE%x*}; H=${SIZE#*x}
DEV=$(eval "$(python3 "$(dirname "$0")/../hostconf.py" --sh)" && echo "$MICROCAM_VIDEO")
case $V in
  mpv) exec mpv --profile=low-latency --untimed --no-cache --video-latency-hacks=yes --framedrop=vo --hwdec=no --vd-lavc-threads=4 \
         --fs --fs-screen-name=$SCR --no-osc --title="microscope mpv" \
         --demuxer-lavf-format=video4linux2 --demuxer-lavf-o=input_format=mjpeg,video_size=$SIZE,framerate=$FPS "av://v4l2:$DEV" ;;
  wl)  exec gst-launch-1.0 -q v4l2src device=$DEV ! image/jpeg,width=$W,height=$H,framerate=$FPS/1 ! queue max-size-buffers=1 leaky=downstream \
         ! jpegdec ! queue max-size-buffers=1 leaky=downstream ! waylandsink fullscreen=true sync=false ;;
  gl)  exec gst-launch-1.0 -q v4l2src device=$DEV ! image/jpeg,width=$W,height=$H,framerate=$FPS/1 ! queue max-size-buffers=1 leaky=downstream \
         ! jpegdec ! queue max-size-buffers=1 leaky=downstream ! glimagesink sync=false ;;
esac
