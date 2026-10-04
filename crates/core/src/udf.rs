//! A small read-only UDF reader (ECMA-167 / OSTA UDF 1.02–2.01), enough to
//! list and read the files of a Windows installer ISO without mounting it.
//!
//! Supports a single type 1 (physical) partition, short/long/embedded
//! allocation descriptors and descriptor continuation extents. UDF 2.5+
//! metadata partitions (Blu-ray) are rejected.

use std::io::{self, Read, Seek, SeekFrom};

const SECTOR: u64 = 2048;
const ANCHOR_SECTOR: u64 = 256;

const TAG_AVDP: u16 = 2;
const TAG_PARTITION: u16 = 5;
const TAG_LOGICAL_VOLUME: u16 = 6;
const TAG_TERMINATOR: u16 = 8;
const TAG_FILE_SET: u16 = 256;
const TAG_FILE_ID: u16 = 257;
const TAG_AED: u16 = 258;
const TAG_FILE_ENTRY: u16 = 261;
const TAG_EXT_FILE_ENTRY: u16 = 266;

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("UDF: {}", msg.into()))
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Checks a descriptor tag (identifier and header checksum) and returns its identifier.
fn tag_id(b: &[u8]) -> Option<u16> {
    if b.len() < 16 {
        return None;
    }
    let sum = b[..16]
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != 4)
        .fold(0u8, |acc, (_, &x)| acc.wrapping_add(x));
    (sum == b[4]).then(|| u16_at(b, 0))
}

/// A run of logical blocks within the partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Extent {
    lbn: u32,
    len: u32,
    /// Allocated but not recorded: reads as zeros.
    sparse: bool,
}

/// Where a file's data lives.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Data {
    Extents(Vec<Extent>),
    Embedded(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    icb: u32,
}

pub struct Udf<R> {
    r: R,
    /// First sector of the partition.
    part_start: u64,
    block: u64,
    root_icb: u32,
}

impl<R: Read + Seek> Udf<R> {
    pub fn open(mut r: R) -> io::Result<Self> {
        let anchor = read_sector(&mut r, ANCHOR_SECTOR, SECTOR)?;
        if tag_id(&anchor) != Some(TAG_AVDP) {
            return Err(bad("no anchor volume descriptor"));
        }
        let vds_len = u32_at(&anchor, 16) as u64;
        let vds_loc = u32_at(&anchor, 20) as u64;

        let (mut part_start, mut block, mut fsd) = (None, SECTOR, None);
        for i in 0..(vds_len / SECTOR).min(64) {
            let d = read_sector(&mut r, vds_loc + i, SECTOR)?;
            match tag_id(&d) {
                Some(TAG_PARTITION) => part_start = Some(u32_at(&d, 188) as u64),
                Some(TAG_LOGICAL_VOLUME) => {
                    block = u32_at(&d, 212) as u64;
                    // long_ad of the File Set Descriptor in Logical Volume Contents Use.
                    fsd = Some(u32_at(&d, 252));
                    let maps = u32_at(&d, 268);
                    let mut off = 440;
                    for _ in 0..maps {
                        let (kind, len) = (d[off], d[off + 1] as usize);
                        if kind != 1 {
                            return Err(bad("only plain (type 1) partitions are supported"));
                        }
                        off += len.max(1);
                    }
                }
                Some(TAG_TERMINATOR) => break,
                _ => {}
            }
        }
        let part_start = part_start.ok_or_else(|| bad("no partition descriptor"))?;
        let fsd = fsd.ok_or_else(|| bad("no logical volume descriptor"))?;
        if block != SECTOR {
            return Err(bad(format!("unsupported block size {block}")));
        }
        let mut udf = Self {
            r,
            part_start,
            block,
            root_icb: 0,
        };
        let fs = udf.read_block(fsd)?;
        if tag_id(&fs) != Some(TAG_FILE_SET) {
            return Err(bad("no file set descriptor"));
        }
        udf.root_icb = u32_at(&fs, 404); // Root Directory ICB long_ad, lbn
        Ok(udf)
    }

    fn read_block(&mut self, lbn: u32) -> io::Result<Vec<u8>> {
        read_sector(&mut self.r, self.part_start + lbn as u64, self.block)
    }

    /// Parses a (Extended) File Entry: is-directory, size and data location.
    fn file_entry(&mut self, icb: u32) -> io::Result<(bool, u64, Data)> {
        let fe = self.read_block(icb)?;
        let (ea_off, base) = match tag_id(&fe) {
            Some(TAG_FILE_ENTRY) => (168, 176),
            Some(TAG_EXT_FILE_ENTRY) => (208, 216),
            _ => return Err(bad(format!("bad file entry at block {icb}"))),
        };
        let file_type = fe[16 + 11];
        let flags = u16_at(&fe, 16 + 18);
        let size = u64_at(&fe, 56);
        let l_ea = u32_at(&fe, ea_off) as usize;
        let l_ad = u32_at(&fe, ea_off + 4) as usize;
        let start = base + l_ea;
        let ads = fe
            .get(start..start + l_ad)
            .ok_or_else(|| bad("allocation descriptors overrun the file entry"))?
            .to_vec();
        let data = match flags & 7 {
            0 => Data::Extents(self.collect_extents(&ads, 8)?),
            1 => Data::Extents(self.collect_extents(&ads, 16)?),
            3 => Data::Embedded(ads),
            other => return Err(bad(format!("unsupported allocation type {other}"))),
        };
        Ok((file_type == 4, size, data))
    }

    /// Reads short (8-byte) or long (16-byte) allocation descriptors,
    /// following continuation extents.
    fn collect_extents(&mut self, ads: &[u8], width: usize) -> io::Result<Vec<Extent>> {
        let mut out = Vec::new();
        let mut buf = ads.to_vec();
        for _ in 0..1024 {
            let mut next = None;
            for ad in buf.chunks_exact(width) {
                let raw = u32_at(ad, 0);
                let (kind, len) = (raw >> 30, raw & 0x3FFF_FFFF);
                if len == 0 {
                    break;
                }
                let lbn = u32_at(ad, 4);
                match kind {
                    3 => {
                        next = Some(lbn);
                        break;
                    }
                    0 => out.push(Extent {
                        lbn,
                        len,
                        sparse: false,
                    }),
                    _ => out.push(Extent {
                        lbn,
                        len,
                        sparse: true,
                    }),
                }
            }
            let Some(lbn) = next else { return Ok(out) };
            // Allocation Extent Descriptor: 24-byte header, then more descriptors.
            let aed = self.read_block(lbn)?;
            if tag_id(&aed) != Some(TAG_AED) {
                return Err(bad("bad allocation extent descriptor"));
            }
            let len = u32_at(&aed, 20) as usize;
            buf = aed
                .get(24..24 + len)
                .ok_or_else(|| bad("allocation extent overruns its block"))?
                .to_vec();
        }
        Err(bad("too many allocation extents"))
    }

    fn read_data(&mut self, size: u64, data: &Data) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(size.min(16 << 20) as usize);
        self.reader_for(size, data).read_to_end(&mut out)?;
        Ok(out)
    }

    fn reader_for(&mut self, size: u64, data: &Data) -> FileReader<'_, R> {
        FileReader {
            udf: self,
            data: data.clone(),
            remaining: size,
            extent: 0,
            offset: 0,
        }
    }

    fn list(&mut self, icb: u32) -> io::Result<Vec<Entry>> {
        let (is_dir, size, data) = self.file_entry(icb)?;
        if !is_dir {
            return Err(bad("not a directory"));
        }
        if size > 64 << 20 {
            return Err(bad("directory too large"));
        }
        let raw = self.read_data(size, &data)?;
        let mut out = Vec::new();
        let mut off = 0;
        while off + 38 <= raw.len() {
            let fid = &raw[off..];
            if tag_id(fid) != Some(TAG_FILE_ID) {
                return Err(bad("bad file identifier descriptor"));
            }
            let chars = fid[18];
            let l_fi = fid[19] as usize;
            let icb = u32_at(fid, 24);
            let l_iu = u16_at(fid, 36) as usize;
            let total = (38 + l_iu + l_fi + 3) & !3;
            let name_bytes = fid
                .get(38 + l_iu..38 + l_iu + l_fi)
                .ok_or_else(|| bad("file name overruns the directory"))?;
            off += total;
            // Skip deleted (bit 2) and parent (bit 3) entries.
            if chars & 0b1100 != 0 {
                continue;
            }
            let (is_dir, size, _) = self.file_entry(icb)?;
            out.push(Entry {
                name: decode_dstring(name_bytes),
                is_dir,
                size,
                icb,
            });
        }
        Ok(out)
    }

    pub fn root(&self) -> Entry {
        Entry {
            name: String::new(),
            is_dir: true,
            size: 0,
            icb: self.root_icb,
        }
    }

    pub fn read_dir(&mut self, dir: &Entry) -> io::Result<Vec<Entry>> {
        self.list(dir.icb)
    }

    /// Finds `a/b/c`, ignoring case like Windows does.
    pub fn lookup(&mut self, path: &str) -> io::Result<Option<Entry>> {
        let mut cur = self.root();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if !cur.is_dir {
                return Ok(None);
            }
            match self
                .read_dir(&cur)?
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(part))
            {
                Some(e) => cur = e,
                None => return Ok(None),
            }
        }
        Ok(Some(cur))
    }

    /// Every file and directory, depth first, as (`a/b/c`, entry).
    pub fn walk(&mut self) -> io::Result<Vec<(String, Entry)>> {
        let mut out = Vec::new();
        let mut stack = vec![(String::new(), self.root())];
        while let Some((prefix, dir)) = stack.pop() {
            for e in self.read_dir(&dir)? {
                let path = if prefix.is_empty() {
                    e.name.clone()
                } else {
                    format!("{prefix}/{}", e.name)
                };
                if e.is_dir {
                    stack.push((path.clone(), e.clone()));
                }
                out.push((path, e));
            }
            if out.len() > 1_000_000 {
                return Err(bad("too many files"));
            }
        }
        Ok(out)
    }

    /// A reader over a file's contents.
    pub fn open_file(&mut self, entry: &Entry) -> io::Result<impl Read + '_> {
        let (is_dir, size, data) = self.file_entry(entry.icb)?;
        if is_dir {
            return Err(bad(format!("{} is a directory", entry.name)));
        }
        Ok(self.reader_for(size, &data))
    }
}

fn read_sector(r: &mut (impl Read + Seek), sector: u64, size: u64) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; size as usize];
    r.seek(SeekFrom::Start(sector * size))?;
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// OSTA compressed unicode: first byte 8 (one byte per char) or 16 (UCS-2 big endian).
fn decode_dstring(b: &[u8]) -> String {
    match b.split_first() {
        Some((8, rest)) => rest.iter().map(|&c| c as char).collect(),
        Some((16, rest)) => {
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::new(),
    }
}

pub struct FileReader<'a, R> {
    udf: &'a mut Udf<R>,
    data: Data,
    remaining: u64,
    extent: usize,
    /// Bytes already consumed from the current extent.
    offset: u64,
}

impl<R: Read + Seek> Read for FileReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 || buf.is_empty() {
            return Ok(0);
        }
        let extents = match &self.data {
            Data::Embedded(bytes) => {
                let start = self.offset as usize;
                let n = (bytes.len() - start.min(bytes.len()))
                    .min(buf.len())
                    .min(self.remaining as usize);
                buf[..n].copy_from_slice(&bytes[start..start + n]);
                self.offset += n as u64;
                self.remaining -= n as u64;
                return Ok(n);
            }
            Data::Extents(e) => e,
        };
        let Some(ext) = extents.get(self.extent).copied() else {
            return Err(bad("file is shorter than its recorded size"));
        };
        let left_in_extent = ext.len as u64 - self.offset;
        let n = (buf.len() as u64).min(left_in_extent).min(self.remaining) as usize;
        if ext.sparse {
            buf[..n].fill(0);
        } else {
            let pos = (self.udf.part_start + ext.lbn as u64) * self.udf.block + self.offset;
            self.udf.r.seek(SeekFrom::Start(pos))?;
            self.udf.r.read_exact(&mut buf[..n])?;
        }
        self.offset += n as u64;
        self.remaining -= n as u64;
        if self.offset >= ext.len as u64 {
            self.extent += 1;
            self.offset = 0;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_udf;
    use std::fs::File;

    #[test]
    fn decodes_names() {
        assert_eq!(decode_dstring(b"\x08BOOTMGR"), "BOOTMGR");
        assert_eq!(decode_dstring(b"\x10\x00\xDC\x00n"), "Ün");
        assert_eq!(decode_dstring(b""), "");
    }

    #[test]
    fn reads_a_real_udf_image() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("sources")).unwrap();
        std::fs::create_dir_all(src.join("efi/boot")).unwrap();
        std::fs::write(src.join("bootmgr"), b"boot manager").unwrap();
        std::fs::write(src.join("efi/boot/bootx64.efi"), b"efi").unwrap();
        let big: Vec<u8> = (0..3_000_017u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(src.join("sources/install.wim"), &big).unwrap();
        std::fs::write(src.join("Ünïcode name.txt"), "ü").unwrap();

        let iso = dir.path().join("out.iso");
        if !make_udf(&src, &iso) {
            eprintln!("skipping: no UDF image tool (hdiutil / genisoimage / mkisofs)");
            return;
        }

        let mut udf = Udf::open(File::open(&iso).unwrap()).unwrap();
        let wim = udf.lookup("SOURCES/Install.WIM").unwrap().unwrap();
        assert!(!wim.is_dir);
        assert_eq!(wim.size, big.len() as u64);
        let mut got = Vec::new();
        udf.open_file(&wim).unwrap().read_to_end(&mut got).unwrap();
        assert!(got == big, "install.wim contents differ");

        let mut boot = String::new();
        let e = udf.lookup("bootmgr").unwrap().unwrap();
        udf.open_file(&e)
            .unwrap()
            .read_to_string(&mut boot)
            .unwrap();
        assert_eq!(boot, "boot manager");

        let paths: Vec<String> = udf.walk().unwrap().into_iter().map(|(p, _)| p).collect();
        for want in [
            "bootmgr",
            "efi",
            "efi/boot",
            "efi/boot/bootx64.efi",
            "sources",
            "sources/install.wim",
            "Ünïcode name.txt",
        ] {
            assert!(
                paths.iter().any(|p| p == want),
                "missing {want} in {paths:?}"
            );
        }
        assert!(udf.lookup("nope/nothing").unwrap().is_none());
    }

    #[test]
    fn rejects_non_udf() {
        let data = vec![0u8; 600 * 2048];
        assert!(Udf::open(io::Cursor::new(data)).is_err());
    }
}
