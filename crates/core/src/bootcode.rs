//! Legacy BIOS boot code for Windows installer drives.
//!
//! The machine code comes from `crates/core/boot/*.S` (our own, GPL-3.0-or-later),
//! assembled by `crates/core/boot/build.sh`. UEFI firmware ignores all of it.

use std::io::{self, SeekFrom};

use crate::blockio::{round_up, AlignedBuf, ALIGN};
use crate::write_raw::Target;

/// MBR boot code: loads the active partition's boot sector. At most 440 bytes.
pub const MBR: &[u8] = include_bytes!("../boot/mbr.bin");

/// FAT32 boot sector code (first 512 bytes) followed by its stage 2 (1024 bytes).
const VBR: &[u8] = include_bytes!("../boot/vbr.bin");

/// Reserved sector where stage 2 lives (the boot code reads sectors 2 and 3).
pub const STAGE2_SECTOR: u64 = 2;
const STAGE2_SECTORS: u64 = 2;

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Puts the boot code into a FAT32 boot sector, keeping the formatter's BPB.
pub fn patch_boot_sector(sector: &mut [u8], partition_start_lba: u32) -> io::Result<()> {
    if sector.len() < 512 || &sector[0x52..0x5a] != b"FAT32   " {
        return Err(invalid("not a FAT32 boot sector"));
    }
    sector[0..3].copy_from_slice(&VBR[0..3]);
    sector[0x5a..0x1fe].copy_from_slice(&VBR[0x5a..0x1fe]);
    sector[0x1fe..0x200].copy_from_slice(&[0x55, 0xaa]);
    // What Windows' own format writes; some boot code checks it.
    sector[3..11].copy_from_slice(b"MSDOS5.0");
    // Hidden sectors: where the partition starts, which the boot code adds to every read.
    sector[0x1c..0x20].copy_from_slice(&partition_start_lba.to_le_bytes());
    sector[0x40] = 0x80;
    Ok(())
}

/// Installs the FAT32 boot code into the volume at `start` (bytes) on `dev`:
/// boot sector, its backup, and stage 2 in the reserved sectors.
pub fn install_vbr(dev: &mut impl Target, start: u64, sector: u64) -> io::Result<()> {
    let span = round_up(sector, ALIGN as u64) as usize;
    let mut buf = AlignedBuf::new(span);
    dev.seek(SeekFrom::Start(start))?;
    dev.read_exact(&mut buf[..sector as usize])?;

    let bs = &mut buf[..sector as usize];
    let reserved = u16::from_le_bytes([bs[0x0e], bs[0x0f]]) as u64;
    let fs_info = u16::from_le_bytes([bs[0x30], bs[0x31]]) as u64;
    let backup = u16::from_le_bytes([bs[0x32], bs[0x33]]) as u64;
    let stage2 = STAGE2_SECTOR..STAGE2_SECTOR + STAGE2_SECTORS;
    if reserved < stage2.end || stage2.contains(&fs_info) || stage2.contains(&backup) {
        return Err(invalid(
            "the FAT32 layout has no room for the BIOS boot code",
        ));
    }
    patch_boot_sector(bs, (start / sector) as u32)?;

    for at in [0, backup] {
        dev.seek(SeekFrom::Start(start + at * sector))?;
        dev.write_all(&buf[..sector as usize])?;
    }

    let len = round_up(VBR.len() as u64 - 512, sector) as usize;
    let mut stage = AlignedBuf::new(round_up(len as u64, ALIGN as u64) as usize);
    stage[..VBR.len() - 512].copy_from_slice(&VBR[512..]);
    dev.seek(SeekFrom::Start(start + STAGE2_SECTOR * sector))?;
    dev.write_all(&stage[..len])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_fits_its_slots() {
        assert!(MBR.len() <= 440);
        assert_eq!(VBR.len(), 0x600);
        assert_eq!(&VBR[0..3], &[0xeb, 0x58, 0x90]);
        assert_eq!(&VBR[0x1fe..0x200], &[0x55, 0xaa]);
    }

    #[test]
    fn keeps_the_bpb() {
        let mut s = vec![0u8; 512];
        s[0x0b..0x0d].copy_from_slice(&512u16.to_le_bytes());
        s[0x52..0x5a].copy_from_slice(b"FAT32   ");
        s[0x47..0x52].copy_from_slice(b"CCCOMA_X64F");
        patch_boot_sector(&mut s, 2048).unwrap();
        assert_eq!(&s[0x0b..0x0d], &512u16.to_le_bytes());
        assert_eq!(&s[0x47..0x52], b"CCCOMA_X64F");
        assert_eq!(&s[0x1c..0x20], &2048u32.to_le_bytes());
        assert_eq!(s[0x40], 0x80);
        assert_eq!(&s[0x5a..0x1fe], &VBR[0x5a..0x1fe]);
        assert!(patch_boot_sector(&mut [0u8; 512], 0).is_err());
    }
}
