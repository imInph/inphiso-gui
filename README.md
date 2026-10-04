# inphiso

Flash Windows and Linux images to USB drives, on Windows, macOS and Linux.

inphiso is the desktop successor to the [inphiso script](https://github.com/imInph/inphiso). It
handles the stuff that usually breaks: Windows ISOs from macOS or Linux, oversized
`install.wim` files, and the FAT32 4 GiB limit that trips up `dd` and plain file copies.

> **1.0:** Linux ISOs are tested on real USB drives from macOS. Windows ISOs and the Windows
> and Linux versions of the app are newer; if something goes wrong,
> [open an issue](https://github.com/imInph/inphiso-gui/issues). As with any flashing tool,
> double-check the drive before you flash.

## What it does

- Pick a local `.iso` / `.img` (plain or `.xz` / `.gz` / `.bz2` / `.zst`), drop one on the
  window, or paste an HTTPS download link (downloads resume if interrupted).
- Checks the image's SHA-256 against a published checksum (`SHA256SUMS`, `<file>.sha256`,
  Fedora `CHECKSUM` files) found next to it, or one you paste.
- Lists removable drives only. The disk your system runs from is never listed, even with
  "Show all drives" on (it's traced through LVM and LUKS on Linux).
- **Linux and hybrid ISOs, disk images:** written byte for byte, decompressing on the fly.
- **Windows installer ISOs:** detected automatically, written as a FAT32 drive (MBR or GPT),
  with `install.wim` split into `.swm` parts when it's over 4 GiB. Windows Setup puts them
  back together by itself. MBR drives boot on both UEFI and legacy BIOS PCs, using inphiso's
  own boot code (`crates/core/boot/`).
- Verifies the drive by reading it back, then ejects it.
- Asks for your password (or UAC) only for the write itself. The app never runs as admin.

## Boot support

| Image | Legacy BIOS | UEFI |
|---|---|---|
| Linux / hybrid ISO | depends on the ISO | depends on the ISO |
| Windows ISO | yes (MBR, the default) | yes |

## Installing

| Platform | Architectures | Package |
|---|---|---|
| Windows 10/11 | x64, arm64 | `inphiso_<version>_<arch>-setup.exe` |
| macOS 11+ | Apple Silicon, Intel | `inphiso_<version>_<arch>.dmg` |
| Linux | x64, arm64 | `.deb`, `.rpm`, `.AppImage`, `.tar.gz` |

Builds aren't code-signed yet, so the OS will warn you the first time:

- **Windows:** the installer is one click. It installs for your user only (no admin needed)
  and opens inphiso when it's done. If SmartScreen says "Windows protected your PC", choose
  *More info → Run anyway*.
- **macOS:** open the `.dmg` and drag inphiso to Applications. On first launch macOS will say
  it can't verify the developer: open *System Settings → Privacy & Security* and choose
  *Open Anyway*.

### Linux

Pick your distro. The download commands fetch the latest release and pick
x64 or arm64 to match your machine.

<details>
<summary><b>Debian, Ubuntu, Linux Mint, Pop!_OS</b> (.deb)</summary>

```bash
VERSION=$(curl -fsSL https://api.github.com/repos/imInph/inphiso-gui/releases/latest | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p')
ARCH=$(dpkg --print-architecture)   # amd64 or arm64
curl -fLO "https://github.com/imInph/inphiso-gui/releases/download/v$VERSION/inphiso_${VERSION}_${ARCH}.deb"
sudo apt install "./inphiso_${VERSION}_${ARCH}.deb"
```

`apt` also installs `wimtools` (for Windows ISOs) and `pkexec`.

</details>

<details>
<summary><b>Fedora, RHEL, AlmaLinux, Rocky Linux</b> (.rpm)</summary>

```bash
VERSION=$(curl -fsSL https://api.github.com/repos/imInph/inphiso-gui/releases/latest | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p')
ARCH=$(uname -m)   # x86_64 or aarch64
curl -fLO "https://github.com/imInph/inphiso-gui/releases/download/v$VERSION/inphiso-${VERSION}-1.${ARCH}.rpm"
sudo dnf install "./inphiso-${VERSION}-1.${ARCH}.rpm"
```

On RHEL and its rebuilds, `wimlib-utils` (for Windows ISOs) comes from
[EPEL](https://docs.fedoraproject.org/en-US/epel/).

</details>

<details>
<summary><b>openSUSE</b> (.rpm)</summary>

```bash
VERSION=$(curl -fsSL https://api.github.com/repos/imInph/inphiso-gui/releases/latest | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p')
ARCH=$(uname -m)   # x86_64 or aarch64
curl -fLO "https://github.com/imInph/inphiso-gui/releases/download/v$VERSION/inphiso-${VERSION}-1.${ARCH}.rpm"
sudo zypper install --allow-unsigned-rpm "./inphiso-${VERSION}-1.${ARCH}.rpm"
sudo zypper install wimtools   # for Windows ISOs
```

</details>

<details>
<summary><b>Arch Linux, Manjaro, EndeavourOS</b> (build with makepkg)</summary>

Builds the latest commit into a proper pacman package (remove it with `sudo pacman -R inphiso-git`):

```bash
sudo pacman -S --needed git base-devel
git clone https://github.com/imInph/inphiso-gui.git
cd inphiso-gui/packaging/aur/inphiso-git
makepkg -si
sudo pacman -S --needed wimlib   # for Windows ISOs
```

inphiso isn't on the AUR yet. Once it is, `yay -S inphiso-bin` (prebuilt) or
`yay -S inphiso` (from source) will work too.

</details>

<details>
<summary><b>Any other distro</b> (AppImage or .tar.gz)</summary>

The AppImage runs without installing anything:

```bash
VERSION=$(curl -fsSL https://api.github.com/repos/imInph/inphiso-gui/releases/latest | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p')
ARCH=$(uname -m | sed 's/x86_64/amd64/')   # amd64 or aarch64
curl -fLO "https://github.com/imInph/inphiso-gui/releases/download/v$VERSION/inphiso_${VERSION}_${ARCH}.AppImage"
chmod +x "inphiso_${VERSION}_${ARCH}.AppImage"
"./inphiso_${VERSION}_${ARCH}.AppImage"
```

Or the `.tar.gz`, which runs in place or installs to `/opt/inphiso` with a menu entry:

```bash
VERSION=$(curl -fsSL https://api.github.com/repos/imInph/inphiso-gui/releases/latest | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p')
ARCH=$(uname -m | sed 's/x86_64/amd64/; s/aarch64/arm64/')   # amd64 or arm64
curl -fL "https://github.com/imInph/inphiso-gui/releases/download/v$VERSION/inphiso_${VERSION}_${ARCH}.tar.gz" | tar xz
cd "inphiso_${VERSION}_${ARCH}"
sudo ./install.sh   # or just ./inphiso; remove later with sudo ./install.sh --uninstall
```

Both need WebKitGTK 4.1 and pkexec from your distro, plus `wimtools` / `wimlib` to
flash Windows ISOs whose `install.wim` is over 4 GB.

</details>

## Building

Requirements: Rust 1.88+, Node 20+, and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
cd app
npm install
npm run tauri dev
```

`npm run dev` alone serves the UI in a browser with a mock backend, handy for UI work.

Before committing, run everything CI runs:

```bash
./scripts/check.sh
```

## Layout

```
crates/core       image inspection, decompression, checksums, UDF reader,
                  raw and Windows write pipelines, partitioning
crates/platform   per-OS drive listing, raw device access, elevation, IPC channel
crates/helper     the privileged writer process (the only part that runs as admin)
crates/cli        command-line front end (`inphiso list-drives`)
app/              Tauri + React desktop app
installer/        Windows installer template, DMG background, polkit policy
```

## License

Copyright (C) 2026 imInph

inphiso is free software: you can redistribute it and/or modify it under the terms of the
GNU General Public License as published by the Free Software Foundation, either version 3
of the License, or (at your option) any later version. It is distributed in the hope that it
will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See [LICENSE](LICENSE) for the full text.

The macOS and Windows builds include [wimlib](https://wimlib.net)'s `wimlib-imagex`
(GPLv3, library LGPLv3) unmodified, with its license texts and a link to its source.
