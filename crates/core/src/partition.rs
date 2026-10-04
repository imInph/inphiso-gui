//! Building a one-partition MBR or GPT layout for a Windows installer stick.

use crate::ipc::PartitionScheme;

pub const MIB: u64 = 1 << 20;
/// MBR stores sector counts as u32; with 512-byte sectors that caps at 2 TiB.
const MBR_MAX_SECTORS: u64 = u32::MAX as u64;
const GPT_ENTRIES: u64 = 128;
const GPT_ENTRY_SIZE: u64 = 128;
/// Microsoft Basic Data partition type, as stored on disk (mixed endian).
const BASIC_DATA: [u8; 16] = guid_bytes(
    0xEBD0A0A2,
    0xB9E5,
    0x4433,
    [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7],
);

const fn guid_bytes(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

/// Where the single data partition goes, in bytes from the start of the drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub start: u64,
    pub len: u64,
}

/// Plans a 1 MiB-aligned partition filling the drive.
pub fn plan(drive_size: u64, sector: u64, scheme: PartitionScheme) -> Option<Layout> {
    let start = MIB;
    let end = match scheme {
        PartitionScheme::Mbr => drive_size.min(MBR_MAX_SECTORS * sector),
        PartitionScheme::Gpt => {
            let entries = GPT_ENTRIES * GPT_ENTRY_SIZE;
            // Leave room for the backup entries and header at the end.
            drive_size.checked_sub(entries + sector)?
        }
    };
    let end = end / MIB * MIB;
    (end > start + 64 * MIB).then_some(Layout {
        start,
        len: end - start,
    })
}

/// The partition table as (offset, bytes) chunks to write.
pub fn table(
    layout: Layout,
    drive_size: u64,
    sector: u64,
    scheme: PartitionScheme,
    random: [u8; 36],
) -> Vec<(u64, Vec<u8>)> {
    match scheme {
        PartitionScheme::Mbr => vec![(0, mbr(layout, sector, random[..4].try_into().unwrap()))],
        PartitionScheme::Gpt => gpt(
            layout,
            drive_size,
            sector,
            random[4..20].try_into().unwrap(),
            random[20..36].try_into().unwrap(),
        ),
    }
}

fn mbr_entry(e: &mut [u8], active: bool, kind: u8, start_lba: u64, sectors: u64) {
    e[0] = if active { 0x80 } else { 0 };
    // CHS fields set to "use LBA" maximums.
    e[1..4].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
    e[4] = kind;
    e[5..8].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
    e[8..12].copy_from_slice(&(start_lba.min(MBR_MAX_SECTORS) as u32).to_le_bytes());
    e[12..16].copy_from_slice(&(sectors.min(MBR_MAX_SECTORS) as u32).to_le_bytes());
}

fn mbr(layout: Layout, sector: u64, disk_id: [u8; 4]) -> Vec<u8> {
    let mut b = vec![0u8; sector as usize];
    // Boot code for legacy BIOS: chain-loads the active partition. UEFI ignores it.
    b[..crate::bootcode::MBR.len()].copy_from_slice(crate::bootcode::MBR);
    b[440..444].copy_from_slice(&disk_id);
    // 0x0C: FAT32 with LBA addressing. Active so BIOS-era tools see it as bootable.
    mbr_entry(
        &mut b[446..462],
        true,
        0x0C,
        layout.start / sector,
        layout.len / sector,
    );
    b[510] = 0x55;
    b[511] = 0xAA;
    b
}

fn set_guid_version(mut g: [u8; 16]) -> [u8; 16] {
    g[7] = (g[7] & 0x0F) | 0x40; // version 4 (random), stored little-endian in field 3
    g[8] = (g[8] & 0x3F) | 0x80; // RFC 4122 variant
    g
}

/// Fields both GPT headers share.
struct GptCommon {
    first_usable: u64,
    last_usable: u64,
    disk_guid: [u8; 16],
    entries_crc: u32,
    sector: u64,
}

fn gpt_header(c: &GptCommon, current: u64, backup: u64, entries_lba: u64) -> Vec<u8> {
    let mut h = vec![0u8; c.sector as usize];
    h[0..8].copy_from_slice(b"EFI PART");
    h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    h[12..16].copy_from_slice(&92u32.to_le_bytes());
    h[24..32].copy_from_slice(&current.to_le_bytes());
    h[32..40].copy_from_slice(&backup.to_le_bytes());
    h[40..48].copy_from_slice(&c.first_usable.to_le_bytes());
    h[48..56].copy_from_slice(&c.last_usable.to_le_bytes());
    h[56..72].copy_from_slice(&c.disk_guid);
    h[72..80].copy_from_slice(&entries_lba.to_le_bytes());
    h[80..84].copy_from_slice(&(GPT_ENTRIES as u32).to_le_bytes());
    h[84..88].copy_from_slice(&(GPT_ENTRY_SIZE as u32).to_le_bytes());
    h[88..92].copy_from_slice(&c.entries_crc.to_le_bytes());
    let crc = crc32fast::hash(&h[..92]);
    h[16..20].copy_from_slice(&crc.to_le_bytes());
    h
}

fn gpt(
    layout: Layout,
    drive_size: u64,
    sector: u64,
    disk_guid: [u8; 16],
    part_guid: [u8; 16],
) -> Vec<(u64, Vec<u8>)> {
    let total = drive_size / sector;
    let entry_sectors = (GPT_ENTRIES * GPT_ENTRY_SIZE).div_ceil(sector);
    let first_usable = 2 + entry_sectors;
    let last_usable = total - 2 - entry_sectors;
    let backup_entries = total - 1 - entry_sectors;

    let mut entries = vec![0u8; (entry_sectors * sector) as usize];
    let e = &mut entries[..GPT_ENTRY_SIZE as usize];
    e[0..16].copy_from_slice(&BASIC_DATA);
    e[16..32].copy_from_slice(&set_guid_version(part_guid));
    e[32..40].copy_from_slice(&(layout.start / sector).to_le_bytes());
    e[40..48].copy_from_slice(&((layout.start + layout.len) / sector - 1).to_le_bytes());
    for (i, unit) in "Windows Setup".encode_utf16().enumerate() {
        e[56 + i * 2..58 + i * 2].copy_from_slice(&unit.to_le_bytes());
    }
    let crc = crc32fast::hash(&entries[..(GPT_ENTRIES * GPT_ENTRY_SIZE) as usize]);
    let disk_guid = set_guid_version(disk_guid);

    // Protective MBR: one 0xEE partition covering the disk.
    let mut pmbr = vec![0u8; sector as usize];
    mbr_entry(&mut pmbr[446..462], false, 0xEE, 1, total - 1);
    pmbr[447..450].copy_from_slice(&[0x00, 0x02, 0x00]);
    pmbr[510] = 0x55;
    pmbr[511] = 0xAA;

    let common = GptCommon {
        first_usable,
        last_usable,
        disk_guid,
        entries_crc: crc,
        sector,
    };
    let primary = gpt_header(&common, 1, total - 1, 2);
    let backup = gpt_header(&common, total - 1, 1, backup_entries);
    vec![
        (0, pmbr),
        (sector, primary),
        (2 * sector, entries.clone()),
        (backup_entries * sector, entries),
        ((total - 1) * sector, backup),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }
    fn u64_at(b: &[u8], o: usize) -> u64 {
        u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
    }

    #[test]
    fn plans_aligned_partitions() {
        let l = plan(32 * GB, 512, PartitionScheme::Mbr).unwrap();
        assert_eq!(l.start, MIB);
        assert_eq!((l.start + l.len) % MIB, 0);
        assert!(l.start + l.len <= 32 * GB);
        // MBR can't describe more than 2 TiB.
        let big = plan(4_000 * GB, 512, PartitionScheme::Mbr).unwrap();
        assert!((big.start + big.len) / 512 <= u32::MAX as u64);
        let g = plan(32 * GB, 512, PartitionScheme::Gpt).unwrap();
        assert!(g.start + g.len <= 32 * GB - 16384 - 512);
        assert!(plan(10 * MIB, 512, PartitionScheme::Mbr).is_none());
    }

    #[test]
    fn mbr_describes_the_partition() {
        let l = plan(16 * GB, 512, PartitionScheme::Mbr).unwrap();
        let t = table(l, 16 * GB, 512, PartitionScheme::Mbr, [7; 36]);
        assert_eq!(t.len(), 1);
        let b = &t[0].1;
        assert_eq!(&b[510..], &[0x55, 0xAA]);
        assert_eq!(b[446], 0x80);
        assert_eq!(b[450], 0x0C);
        assert_eq!(u32_at(b, 454) as u64, l.start / 512);
        assert_eq!(u32_at(b, 458) as u64, l.len / 512);
    }

    #[test]
    fn gpt_headers_and_crcs_check_out() {
        for sector in [512u64, 4096] {
            let size = 16 * GB / sector * sector;
            let l = plan(size, sector, PartitionScheme::Gpt).unwrap();
            let t = table(l, size, sector, PartitionScheme::Gpt, [3; 36]);
            let total = size / sector;
            let find = |off: u64| &t.iter().find(|(o, _)| *o == off).unwrap().1;

            let pmbr = find(0);
            assert_eq!(pmbr[450], 0xEE);
            for (hdr_off, entries_lba) in [
                (sector, 2),
                ((total - 1) * sector, total - 1 - 16384 / sector),
            ] {
                let h = find(hdr_off);
                assert_eq!(&h[..8], b"EFI PART");
                let mut copy = h[..92].to_vec();
                copy[16..20].fill(0);
                assert_eq!(crc32fast::hash(&copy), u32_at(h, 16), "header crc");
                assert_eq!(u64_at(h, 72), entries_lba);
                let entries = find(entries_lba * sector);
                assert_eq!(
                    crc32fast::hash(&entries[..16384]),
                    u32_at(h, 88),
                    "entries crc"
                );
                assert_eq!(&entries[..16], &BASIC_DATA);
                assert_eq!(u64_at(entries, 32), l.start / sector);
                assert_eq!(u64_at(entries, 40), (l.start + l.len) / sector - 1);
                // The partition sits inside the usable range.
                assert!(u64_at(entries, 32) >= u64_at(h, 40));
                assert!(u64_at(entries, 40) <= u64_at(h, 48));
            }
        }
    }

    #[test]
    fn basic_data_guid_matches_its_text_form() {
        // EBD0A0A2-B9E5-4433-87C0-68B6B72699C7
        assert_eq!(
            BASIC_DATA,
            [
                0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26,
                0x99, 0xC7
            ]
        );
    }
}
