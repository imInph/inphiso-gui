use std::fs::File;

use crate::macos::parse_plist;
use crate::{run, Result};

#[derive(Default)]
pub struct Guard;

pub fn prepare_and_open(id: &str) -> Result<(File, u64, Guard)> {
    run("diskutil", &["unmountDisk", "force", &format!("/dev/{id}")])?;
    let info = parse_plist(&run("diskutil", &["info", "-plist", id])?)?;
    let sector = info
        .get("DeviceBlockSize")
        .and_then(plist::Value::as_unsigned_integer)
        .unwrap_or(512);
    // The raw (character) device skips the buffer cache and is several times faster.
    let file = File::options()
        .read(true)
        .write(true)
        .open(format!("/dev/r{id}"))?;
    Ok((file, sector, Guard))
}

pub fn after_write(_file: &File) {
    // Disk Arbitration notices the new partition table on its own.
}

pub fn eject(id: &str) -> Result<()> {
    run("diskutil", &["eject", &format!("/dev/{id}")])?;
    Ok(())
}
