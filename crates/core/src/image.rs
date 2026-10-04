//! Working out what an image file is: compression, partition table or ISO
//! file system, El Torito boot entries, and how it should be written.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::blockio::read_full;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    None,
    Xz,
    Gzip,
    Bzip2,
    Zstd,
}

/// How the image should be written. `Unknown` means the user has to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Raw,
    Windows,
    Unknown,
}

/// What the UI shows about an image (matches `ImageInfo` in `app/src/types.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    pub path: String,
    pub name: String,
    pub file_size: u64,
    /// Bytes that land on the drive, if known.
    pub write_size: Option<u64>,
    pub compression: Compression,
    pub kind: Kind,
    pub format: String,
    pub boot: Option<String>,
}

pub fn detect_compression(head: &[u8]) -> Compression {
    match head {
        [0xFD, b'7', b'z', b'X', b'Z', 0x00, ..] => Compression::Xz,
        [0x1F, 0x8B, ..] => Compression::Gzip,
        [b'B', b'Z', b'h', b'1'..=b'9', ..] => Compression::Bzip2,
        [0x28, 0xB5, 0x2F, 0xFD, ..] => Compression::Zstd,
        _ => Compression::None,
    }
}

/// Wraps `file` in the right decompressor.
pub fn decoder(file: File, compression: Compression) -> io::Result<Box<dyn Read + Send>> {
    let r = BufReader::with_capacity(1 << 20, file);
    Ok(match compression {
        Compression::None => Box::new(r),
        Compression::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(r)),
        Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(r)),
        Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(r)),
        Compression::Zstd => Box::new(zstd::stream::read::Decoder::with_buffer(r)?),
    })
}

/// Opens an image for writing: the decompressed byte stream, plus its size if known.
pub fn open(path: &Path) -> io::Result<(Box<dyn Read + Send>, Compression, Option<u64>)> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 6];
    let n = read_full(&mut file, &mut magic)?;
    let compression = detect_compression(&magic[..n]);
    let size = match compression {
        Compression::None => Some(file.metadata()?.len()),
        Compression::Xz => xz_uncompressed_size(&mut file).ok().flatten(),
        _ => None,
    };
    file.seek(SeekFrom::Start(0))?;
    Ok((decoder(file, compression)?, compression, size))
}

/// Reads an xz variable-length integer.
fn read_vli(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for i in 0..9 {
        let b = *buf.get(*pos)?;
        *pos += 1;
        value |= u64::from(b & 0x7F) << (7 * i);
        if b & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// Total uncompressed size of a single-stream .xz file, from its index.
/// Returns `None` for anything unusual (multiple streams, padding).
pub fn xz_uncompressed_size(file: &mut File) -> io::Result<Option<u64>> {
    let len = file.metadata()?.len();
    if len < 32 {
        return Ok(None);
    }
    let mut footer = [0u8; 12];
    file.seek(SeekFrom::Start(len - 12))?;
    file.read_exact(&mut footer)?;
    if &footer[10..12] != b"YZ" {
        return Ok(None);
    }
    let backward = (u64::from(u32::from_le_bytes(footer[4..8].try_into().unwrap())) + 1) * 4;
    if backward > len - 12 || backward > 64 << 20 {
        return Ok(None);
    }
    let mut index = vec![0u8; backward as usize];
    file.seek(SeekFrom::Start(len - 12 - backward))?;
    file.read_exact(&mut index)?;
    if index[0] != 0 {
        return Ok(None);
    }
    let mut pos = 1;
    let Some(records) = read_vli(&index, &mut pos) else {
        return Ok(None);
    };
    let mut total = 0u64;
    for _ in 0..records {
        let (Some(_unpadded), Some(uncompressed)) =
            (read_vli(&index, &mut pos), read_vli(&index, &mut pos))
        else {
            return Ok(None);
        };
        total = total.saturating_add(uncompressed);
    }
    Ok(Some(total))
}

const ISO_SECTOR: usize = 2048;
/// Enough of the image to see the partition table and the ISO volume descriptors.
const PROBE: usize = 64 * 1024;

/// What the first bytes of the (decompressed) image say.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Probe {
    pub mbr: bool,
    pub gpt: bool,
    pub iso9660: bool,
    pub udf: bool,
    /// LBA of the El Torito boot catalog, if there is one.
    pub boot_catalog: Option<u32>,
}

pub(crate) fn probe(head: &[u8]) -> Probe {
    let at = |off: usize, len: usize| head.get(off..off + len);
    let mut p = Probe {
        mbr: at(510, 2) == Some(&[0x55, 0xAA]),
        gpt: at(512, 8) == Some(b"EFI PART"),
        ..Default::default()
    };
    // Volume descriptors start at sector 16 and run until a terminator.
    for sector in 16..(head.len() / ISO_SECTOR) {
        let Some(d) = at(sector * ISO_SECTOR, ISO_SECTOR) else {
            break;
        };
        match &d[1..6] {
            b"CD001" => {
                p.iso9660 = true;
                // Type 0 is a boot record; El Torito names itself and points at the catalog.
                if d[0] == 0 && d[7..30].starts_with(b"EL TORITO SPECIFICATION") {
                    p.boot_catalog = Some(u32::from_le_bytes(d[0x47..0x4B].try_into().unwrap()));
                }
            }
            b"NSR02" | b"NSR03" => p.udf = true,
            b"BEA01" | b"TEA01" | b"BOOT2" | b"CDW02" => {}
            _ => break,
        }
    }
    p
}

/// Platform IDs listed in an El Torito boot catalog: 0x00 is BIOS, 0xEF is UEFI.
pub(crate) fn boot_platforms(catalog: &[u8]) -> (bool, bool) {
    let (mut bios, mut efi) = (false, false);
    let mut mark = |id: u8| match id {
        0x00 => bios = true,
        0xEF => efi = true,
        _ => {}
    };
    // Validation entry: header 0x01, key bytes 0x55 0xAA at the end.
    if catalog.len() < 64 || catalog[0] != 0x01 || catalog[30..32] != [0x55, 0xAA] {
        return (false, false);
    }
    mark(catalog[1]);
    // Section headers (0x90 more follow, 0x91 last) each name a platform.
    for entry in catalog[64..].as_chunks::<32>().0 {
        if matches!(entry[0], 0x90 | 0x91) {
            mark(entry[1]);
        }
        if entry[0] == 0x91 {
            break;
        }
    }
    (bios, efi)
}

fn boot_label(bios: bool, efi: bool) -> Option<String> {
    match (efi, bios) {
        (true, true) => Some("bootable (UEFI + BIOS)".into()),
        (true, false) => Some("bootable (UEFI)".into()),
        (false, true) => Some("bootable (BIOS)".into()),
        (false, false) => None,
    }
}

pub fn inspect(path: &Path) -> io::Result<ImageInfo> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
    }
    let (mut stream, compression, write_size) = open(path)?;
    let mut head = vec![0u8; PROBE];
    let n = read_full(&mut stream, &mut head)?;
    head.truncate(n);
    let p = probe(&head);

    let mut boot = None;
    if let Some(lba) = p.boot_catalog {
        // Only for plain files: seeking into a compressed stream means decompressing it.
        if compression == Compression::None {
            let mut f = File::open(path)?;
            f.seek(SeekFrom::Start(u64::from(lba) * ISO_SECTOR as u64))?;
            let mut cat = vec![0u8; ISO_SECTOR];
            if read_full(&mut f, &mut cat)? == ISO_SECTOR {
                let (bios, efi) = boot_platforms(&cat);
                boot = boot_label(bios, efi);
            }
        }
    }

    let format = if p.udf {
        "UDF"
    } else if p.iso9660 {
        "ISO 9660"
    } else if p.gpt {
        "disk image (GPT)"
    } else if p.mbr {
        "disk image (MBR)"
    } else {
        "raw image"
    };

    // A partition table up front means it's a disk image or hybrid ISO, written as-is.
    // A bare ISO (no partition table) is usually a Windows installer; finding out for
    // sure needs its file listing, so for now ask.
    let kind = if compression != Compression::None || p.mbr || p.gpt {
        Kind::Raw
    } else {
        Kind::Unknown
    };

    Ok(ImageInfo {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        file_size: meta.len(),
        write_size,
        compression,
        kind,
        format: format.into(),
        boot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn magic_bytes() {
        assert_eq!(
            detect_compression(&[0xFD, b'7', b'z', b'X', b'Z', 0]),
            Compression::Xz
        );
        assert_eq!(detect_compression(&[0x1F, 0x8B, 8]), Compression::Gzip);
        assert_eq!(detect_compression(b"BZh9"), Compression::Bzip2);
        assert_eq!(
            detect_compression(&[0x28, 0xB5, 0x2F, 0xFD]),
            Compression::Zstd
        );
        assert_eq!(detect_compression(b"BZhx"), Compression::None);
        assert_eq!(detect_compression(&[]), Compression::None);
    }

    fn sample(len: usize) -> Vec<u8> {
        let mut v: Vec<u8> = (0..len).map(|i| (i / 1000) as u8).collect();
        v[510] = 0x55;
        v[511] = 0xAA;
        v
    }

    fn round_trip(compress: impl Fn(&[u8]) -> Vec<u8>, expect: Compression) {
        let data = sample(300_000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img");
        File::create(&path)
            .unwrap()
            .write_all(&compress(&data))
            .unwrap();
        let (mut r, c, size) = open(&path).unwrap();
        assert_eq!(c, expect);
        if matches!(expect, Compression::None | Compression::Xz) {
            assert_eq!(size, Some(data.len() as u64));
        } else {
            assert_eq!(size, None);
        }
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out, data);

        let info = inspect(&path).unwrap();
        assert_eq!(info.kind, Kind::Raw);
        assert_eq!(info.format, "disk image (MBR)");
    }

    #[test]
    fn decompresses_every_format() {
        round_trip(|d| d.to_vec(), Compression::None);
        round_trip(
            |d| {
                let mut e = liblzma::write::XzEncoder::new(Vec::new(), 1);
                e.write_all(d).unwrap();
                e.finish().unwrap()
            },
            Compression::Xz,
        );
        round_trip(
            |d| {
                let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                e.write_all(d).unwrap();
                e.finish().unwrap()
            },
            Compression::Gzip,
        );
        round_trip(
            |d| {
                let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
                e.write_all(d).unwrap();
                e.finish().unwrap()
            },
            Compression::Bzip2,
        );
        round_trip(|d| zstd::encode_all(d, 1).unwrap(), Compression::Zstd);
    }

    fn iso_head() -> Vec<u8> {
        let mut h = vec![0u8; PROBE];
        let pvd = 16 * ISO_SECTOR;
        h[pvd] = 1;
        h[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
        let br = 17 * ISO_SECTOR;
        h[br] = 0;
        h[br + 1..br + 6].copy_from_slice(b"CD001");
        h[br + 7..br + 30].copy_from_slice(b"EL TORITO SPECIFICATION");
        h[br + 0x47..br + 0x4B].copy_from_slice(&40u32.to_le_bytes());
        let term = 18 * ISO_SECTOR;
        h[term] = 255;
        h[term + 1..term + 6].copy_from_slice(b"CD001");
        h
    }

    #[test]
    fn probes_iso_and_el_torito() {
        let p = probe(&iso_head());
        assert!(p.iso9660 && !p.udf && !p.mbr);
        assert_eq!(p.boot_catalog, Some(40));
    }

    #[test]
    fn probes_udf_bridge() {
        let mut h = iso_head();
        let s = 19 * ISO_SECTOR;
        h[s + 1..s + 6].copy_from_slice(b"NSR02");
        // Terminator at 18 stops the scan in a real ISO; move it past the UDF descriptor.
        h[18 * ISO_SECTOR + 1..18 * ISO_SECTOR + 6].copy_from_slice(b"BEA01");
        let p = probe(&h);
        assert!(p.udf && p.iso9660);
    }

    #[test]
    fn reads_boot_platforms() {
        let mut cat = vec![0u8; ISO_SECTOR];
        cat[0] = 0x01; // validation entry, platform 0 = BIOS
        cat[30] = 0x55;
        cat[31] = 0xAA;
        cat[32] = 0x88; // default entry
        cat[64] = 0x91; // last section header
        cat[65] = 0xEF; // UEFI
        assert_eq!(boot_platforms(&cat), (true, true));
        assert_eq!(boot_label(true, true).unwrap(), "bootable (UEFI + BIOS)");
        assert_eq!(boot_platforms(&[0u8; 64]), (false, false));
    }

    #[test]
    fn bare_iso_needs_a_decision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("win.iso");
        let mut img = iso_head();
        img.resize(PROBE * 2, 0);
        File::create(&path).unwrap().write_all(&img).unwrap();
        let info = inspect(&path).unwrap();
        assert_eq!(info.kind, Kind::Unknown);
        assert_eq!(info.format, "ISO 9660");
        assert_eq!(info.name, "win.iso");
    }
}
