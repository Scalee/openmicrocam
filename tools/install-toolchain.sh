#!/bin/bash
# Install the build toolchain (README «Building»): Rust with the armv7 target (rust-toolchain.toml), zig, cargo-zigbuild,
# and on Debian/Ubuntu the host packages: the arm-linux-gnueabihf cross gcc and binutils (kernel modules, camera-side
# C tools), libclang (bindgen), patchelf (tools/mk-linklibs.sh), xz. Already present parts are kept.
# usage: tools/install-toolchain.sh        (rustup and cargo honour RUSTUP_HOME / CARGO_HOME when set)
set -e
ZV=0.14.1          # zig
CZ=v0.24.0         # cargo-zigbuild
PKGS="gcc-arm-linux-gnueabihf binutils-arm-linux-gnueabihf libclang-dev patchelf xz-utils curl"
export RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup} CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}
export PATH=$CARGO_HOME/bin:$HOME/.local/bin:$PATH
W=$(mktemp -d -p "${TMPDIR:-/tmp}" toolchain.XXXXXX); trap 'rm -rf "$W"' EXIT

if command -v apt-get >/dev/null; then
  missing=$(for p in $PKGS; do dpkg -s "$p" >/dev/null 2>&1 || echo "$p"; done)
  [ -z "$missing" ] || sudo apt-get install -y $missing
else
  echo "not Debian/Ubuntu: install by hand the arm-linux-gnueabihf cross gcc and binutils, libclang, patchelf, xz"
fi
arm-linux-gnueabihf-gcc --version | head -1

if ! command -v rustup >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    -c rustfmt -c clippy -t armv7-unknown-linux-gnueabihf
fi
rustup target add armv7-unknown-linux-gnueabihf
rustc --version

mkdir -p "$HOME/.local/opt" "$HOME/.local/bin"
if [ "$(zig version 2>/dev/null)" != "$ZV" ]; then
  curl -fsSL -o "$W/zig.tar.xz" "https://ziglang.org/download/$ZV/zig-x86_64-linux-$ZV.tar.xz" ||
    curl -fsSL -o "$W/zig.tar.xz" "https://ziglang.org/download/$ZV/zig-linux-x86_64-$ZV.tar.xz"
  mkdir "$W/zig" && tar -xJf "$W/zig.tar.xz" -C "$W/zig" --strip-components=1
  rm -rf "$HOME/.local/opt/zig" && mv "$W/zig" "$HOME/.local/opt/zig"
  ln -sf "$HOME/.local/opt/zig/zig" "$HOME/.local/bin/zig"
fi
zig version

if [ "$(cargo-zigbuild --version 2>/dev/null)" != "cargo-zigbuild ${CZ#v}" ]; then
  curl -fsSL -o "$W/cz.tar.xz" "https://github.com/rust-cross/cargo-zigbuild/releases/download/$CZ/cargo-zigbuild-x86_64-unknown-linux-musl.tar.xz"
  tar -xJf "$W/cz.tar.xz" -C "$W" --strip-components=1 && install -m755 "$W/cargo-zigbuild" "$CARGO_HOME/bin/"
fi
cargo-zigbuild --version
echo "toolchain ready"
