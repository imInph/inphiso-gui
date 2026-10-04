use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use crate::{run, Error, Result};

#[derive(Default)]
pub struct Guard;

/// `_IO(0x12, 95)`: re-read the partition table.
const BLKRRPART: libc::c_ulong = 0x125f;

/// Mounted partitions of `id`, from lsblk's raw output.
fn mounted_parts(id: &str) -> Result<Vec<String>> {
    let out = run("lsblk", &["-nrpo", "NAME,MOUNTPOINT", id])?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(2, ' ');
            let name = it.next()?;
            let mount = it.next().unwrap_or("").trim();
            (!mount.is_empty()).then(|| name.to_string())
        })
        .collect())
}

pub fn sector_size(id: &str) -> u64 {
    logical_sector(id)
}

fn logical_sector(id: &str) -> u64 {
    let name = id.trim_start_matches("/dev/");
    std::fs::read_to_string(format!("/sys/block/{name}/queue/logical_block_size"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(512)
}

fn open(id: &str, direct: bool) -> io::Result<File> {
    // O_EXCL on a block device fails if anything (a mount, LVM, dm-crypt) still holds it.
    let mut flags = libc::O_EXCL;
    if direct {
        // Bypass the page cache so progress reflects what actually reached the drive.
        flags |= libc::O_DIRECT;
    }
    File::options()
        .read(true)
        .write(true)
        .custom_flags(flags)
        .open(id)
}

pub fn prepare_and_open(id: &str) -> Result<(File, u64, Guard)> {
    for part in mounted_parts(id)? {
        run("umount", &["--all-targets", &part])?;
    }
    let file = match open(id, true) {
        Err(e) if e.raw_os_error() == Some(libc::EINVAL) => open(id, false),
        other => other,
    }
    .map_err(|e| match e.raw_os_error() {
        Some(libc::EBUSY) => Error::Refused(format!(
            "{id} is still in use (mounted, or part of LVM or an encrypted volume)."
        )),
        _ => Error::Io(e),
    })?;
    Ok((file, logical_sector(id), Guard))
}

pub fn sync(file: &File) -> io::Result<()> {
    file.sync_all()
}

pub fn after_write(file: &File) {
    unsafe {
        libc::ioctl(file.as_raw_fd(), BLKRRPART as _);
    }
}

pub fn eject(id: &str) -> Result<()> {
    // Desktops auto-mount the freshly written partitions; let go of them first.
    for part in mounted_parts(id)? {
        if run("udisksctl", &["unmount", "-b", &part]).is_err() {
            run("umount", &["--all-targets", &part])?;
        }
    }
    if run("udisksctl", &["power-off", "-b", id]).is_ok() {
        return Ok(());
    }
    run("eject", &[id])?;
    Ok(())
}
