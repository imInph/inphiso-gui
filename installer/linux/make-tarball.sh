#!/usr/bin/env bash
# Packs a portable Linux .tar.gz from a release build.
#   installer/linux/make-tarball.sh <target-triple>
# Writes target/<triple>/release/bundle/tarball/inphiso_<version>_<arch>.tar.gz
# and prints its path.
set -euo pipefail
cd "$(dirname "$0")/../.."

triple=${1:?usage: make-tarball.sh <target-triple>}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
case "$triple" in
  x86_64-*) arch=amd64 ;;
  aarch64-*) arch=arm64 ;;
  *) arch=${triple%%-*} ;;
esac

release="target/$triple/release"
name="inphiso_${version}_${arch}"
out="$release/bundle/tarball"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
dir="$stage/$name"
mkdir -p "$dir" "$out"

install -m 755 "$release/inphiso-app" "$dir/inphiso"
install -m 755 "$release/inphiso-helper" "$dir/inphiso-helper"
install -m 755 installer/linux/install.sh "$dir/install.sh"
install -m 644 installer/linux/com.inphner.inphiso.policy "$dir/"
install -m 644 app/src-tauri/icons/128x128@2x.png "$dir/inphiso.png"
install -m 644 LICENSE "$dir/LICENSE"

cat > "$dir/inphiso.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=inphiso
Comment=Flash Windows and Linux images to USB drives
Exec=inphiso
Icon=inphiso
Categories=Utility;System;
Terminal=false
EOF

cat > "$dir/README.txt" <<EOF
inphiso $version for Linux ($arch)

Run it in place:   ./inphiso
Install it:        sudo ./install.sh          (to /opt/inphiso, with a menu entry)
Uninstall:         sudo ./install.sh --uninstall

Keep inphiso-helper next to inphiso: it's the part that writes to the drive,
started through pkexec when you flash. You'll need:
  - pkexec (polkit), for the password prompt
  - WebKitGTK 4.1 (libwebkit2gtk-4.1)
  - wimtools / wimlib-utils, to flash Windows ISOs whose install.wim is over 4 GB

Source and license (GPL-3.0-or-later): https://github.com/imInph/inphiso-gui
EOF

tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
echo "$out/$name.tar.gz"
