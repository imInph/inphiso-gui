# inphiso

Flash Windows and Linux images to USB drives, on Windows, macOS and Linux.

inphiso is the desktop successor to the [inphiso script](https://github.com/imInph/inphiso). It
handles the stuff that usually breaks: Windows ISOs from macOS or Linux, oversized
`install.wim` files, and the FAT32 4 GiB limit that trips up `dd` and plain file copies.

> **Status:** pre-release. Everything below is implemented and tested against disk image
> files; real-hardware testing is still in progress. Double-check the drive before you flash.

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
| Linux | x64, arm64 | `.deb`, `.rpm`, `.AppImage` |

Builds aren't code-signed yet, so the OS will warn you the first time:

- **Windows:** the installer is one click. It installs for your user only (no admin needed)
  and opens inphiso when it's done. If SmartScreen says "Windows protected your PC", choose
  *More info → Run anyway*.
- **macOS:** open the `.dmg` and drag inphiso to Applications. On first launch macOS will say
  it can't verify the developer: open *System Settings → Privacy & Security* and choose
  *Open Anyway*.
- **Linux:** the `.deb` and `.rpm` pull in `wimtools` / `wimlib-utils` (for Windows ISOs) and
  pkexec. With the AppImage, install `wimtools` yourself if you flash Windows ISOs.

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
