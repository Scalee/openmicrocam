#!/bin/sh
# Put a test image on the laptop screen (as the GNOME wallpaper: GNOME only; the camera looks at the laptop screen centre)
# and restore the real one. usage: screen.sh show <png>   |   screen.sh restore
S=$(eval "$(python3 "$(dirname "$0")/../hostconf.py" --sh)" && echo "$MICROCAM_LOGS")/wallpaper; mkdir -p "$S"
case "$1" in
  show)
    [ -f "$S"/uri.txt ] || { gsettings get org.gnome.desktop.background picture-uri > "$S"/uri.txt; gsettings get org.gnome.desktop.background picture-uri-dark > "$S"/uri-dark.txt; gsettings get org.gnome.desktop.background picture-options > "$S"/options.txt; }
    gsettings set org.gnome.desktop.background picture-uri "file://$(realpath "$2")"; gsettings set org.gnome.desktop.background picture-uri-dark "file://$(realpath "$2")"; gsettings set org.gnome.desktop.background picture-options 'scaled' ;;
  restore)
    [ -f "$S"/uri.txt ] || { echo "nothing saved"; exit 1; }
    gsettings set org.gnome.desktop.background picture-uri "$(cat "$S"/uri.txt | tr -d "'")"; gsettings set org.gnome.desktop.background picture-uri-dark "$(cat "$S"/uri-dark.txt | tr -d "'")"; gsettings set org.gnome.desktop.background picture-options "$(cat "$S"/options.txt | tr -d "'")"; rm -f "$S"/uri.txt "$S"/uri-dark.txt "$S"/options.txt ;;
  *) echo "usage: screen.sh show <png> | restore"; exit 1 ;;
esac
