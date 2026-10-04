# inphiso

Flash Windows and Linux images to USB drives, on Windows, macOS and Linux.

inphiso is the desktop successor to the [inphiso script](https://github.com/imInph/inphiso). It
handles the stuff that usually breaks: Windows ISOs from macOS or Linux, oversized
`install.wim` files, and the FAT32 4 GiB limit that trips up `dd` and plain file copies.

> **Status:** early development. Nothing here is ready to flash a drive yet.

## Planned for v0.1

- Pick a local `.iso` / `.img` (plain or `.xz` / `.gz` / `.bz2` / `.zst`) or paste a download link
- SHA-256 check against a published checksum
- Removable drives only by default; the system disk is never listed
- Linux and hybrid ISOs written raw; Windows ISOs written as FAT32 with `install.wim` split
  into `.swm` parts when it's over 4 GiB
- Verify after writing, then eject

| Platform | Architectures | Package |
|---|---|---|
| Windows 10/11 | x64, arm64 | one-click installer |
| macOS 11+ | Apple Silicon, Intel | `.dmg` |
| Linux | x64, arm64 | AppImage, `.deb`, `.rpm` |

## Boot support

| Image | Legacy BIOS | UEFI |
|---|---|---|
| Linux / hybrid ISO | depends on the ISO | depends on the ISO |
| Windows ISO | not in v0.1 | yes |

## Building

Requirements: Rust (stable), Node 20+, and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
cd app
npm install
npm run tauri dev
```

## Layout

```
crates/core       image inspection, decompression, checksums, write pipelines
crates/platform   per-OS drive listing, raw device access, elevation
crates/helper     the privileged writer process
crates/cli        command-line front end
app/              Tauri + React desktop app
```

## License

MIT, see [LICENSE](LICENSE).
