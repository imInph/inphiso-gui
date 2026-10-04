#!/bin/sh
# Installs inphiso from the .tar.gz into /opt/inphiso (run as root),
# or removes it again with --uninstall.
set -eu

PREFIX=/opt/inphiso
HERE=$(cd "$(dirname "$0")" && pwd)

if [ "$(id -u)" -ne 0 ]; then
  echo "Run this as root: sudo $0 $*" >&2
  exit 1
fi

if [ "${1:-}" = "--uninstall" ]; then
  rm -rf "$PREFIX"
  rm -f /usr/local/bin/inphiso
  rm -f /usr/share/applications/inphiso.desktop
  rm -f /usr/share/icons/hicolor/256x256/apps/inphiso.png
  rm -f /usr/share/polkit-1/actions/com.inphner.inphiso.policy
  echo "inphiso removed."
  exit 0
fi

install -d "$PREFIX"
install -m 755 "$HERE/inphiso" "$PREFIX/inphiso"
install -m 755 "$HERE/inphiso-helper" "$PREFIX/inphiso-helper"
install -m 644 "$HERE/LICENSE" "$PREFIX/LICENSE"
ln -sf "$PREFIX/inphiso" /usr/local/bin/inphiso

install -d /usr/share/applications /usr/share/icons/hicolor/256x256/apps
install -m 644 "$HERE/inphiso.png" /usr/share/icons/hicolor/256x256/apps/inphiso.png
sed "s|^Exec=.*|Exec=$PREFIX/inphiso|" "$HERE/inphiso.desktop" > /usr/share/applications/inphiso.desktop

# The password prompt names inphiso only if the policy points at the real helper path.
install -d /usr/share/polkit-1/actions
sed "s|/usr/bin/inphiso-helper|$PREFIX/inphiso-helper|" "$HERE/com.inphner.inphiso.policy" \
  > /usr/share/polkit-1/actions/com.inphner.inphiso.policy

if ! command -v wimlib-imagex >/dev/null 2>&1; then
  echo "Note: install wimtools (wimlib) to flash Windows ISOs with install.wim over 4 GB."
fi
echo "inphiso installed. Start it from your app menu or run: inphiso"
