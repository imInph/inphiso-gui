use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

use crate::macos::parse_plist;
use crate::{run, Error, Result};

#[derive(Default)]
pub struct Guard;

pub fn unmount(id: &str) -> Result<()> {
    run("diskutil", &["unmountDisk", "force", &format!("/dev/{id}")])?;
    Ok(())
}

pub fn sector_size(id: &str) -> u64 {
    run("diskutil", &["info", "-plist", id])
        .ok()
        .and_then(|out| parse_plist(&out).ok())
        .and_then(|info| info.get("DeviceBlockSize")?.as_unsigned_integer())
        .unwrap_or(512)
}

/// Opens the raw device with Apple's `authopen`, which asks for an
/// administrator password and passes the open file back over a socket.
pub fn authopen(id: &str) -> Result<File> {
    // The raw (character) device skips the buffer cache and is several times faster.
    authopen_path(&format!("/dev/r{id}"))
}

fn authopen_path(device: &str) -> Result<File> {
    let (ours, theirs) = UnixStream::pair()?;
    let mut child = Command::new("/usr/libexec/authopen")
        .args(["-stdoutpipe", "-o", &libc::O_RDWR.to_string(), device])
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(theirs)))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| Error::Spawn {
            cmd: "authopen",
            source,
        })?;
    let mut stderr = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut stderr);
    }
    let status = child.wait()?;
    if status.success() {
        if let Some(file) = crate::channel::recv_file(ours.as_raw_fd())? {
            return Ok(file);
        }
    }
    let stderr = stderr.trim();
    // A dismissed password prompt exits non-zero without saying much.
    if stderr.is_empty() || stderr.to_lowercase().contains("cancel") {
        return Err(Error::AuthCancelled);
    }
    Err(Error::Refused(format!(
        "macOS didn't allow access to {device}: {stderr}"
    )))
}

/// Used when no device was handed over (development, or running as root).
pub fn prepare_and_open(id: &str) -> Result<(File, u64, Guard)> {
    unmount(id)?;
    let device = format!("/dev/r{id}");
    let file = File::options()
        .read(true)
        .write(true)
        .open(&device)
        .map_err(|e| Error::Refused(format!("couldn't open {device}: {e}")))?;
    Ok((file, sector_size(id), Guard))
}

/// `File::sync_all` uses `F_FULLFSYNC` on macOS, which raw disk devices reject.
/// Raw devices aren't cached, so `fsync` plus asking the drive to flush its own
/// cache is the right thing.
pub fn sync(file: &File) -> std::io::Result<()> {
    /// `_IO('d', 22)`
    const DKIOCSYNCHRONIZECACHE: libc::c_ulong = 0x2000_6416;
    let fd = file.as_raw_fd();
    if unsafe { libc::fsync(fd) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Not every device (or a plain file in tests) supports it; fsync is what counts.
    unsafe {
        libc::ioctl(fd, DKIOCSYNCHRONIZECACHE);
    }
    Ok(())
}

pub fn after_write(_file: &File) {
    // Disk Arbitration notices the new partition table on its own.
}

pub fn eject(id: &str) -> Result<()> {
    run("diskutil", &["eject", &format!("/dev/{id}")])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    /// authopen opens files the caller may already open without prompting, so a
    /// temp file exercises the whole hand-over without a password dialog.
    #[test]
    fn authopen_hands_back_a_writable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.bin");
        std::fs::write(&path, b"before").unwrap();
        let mut f = authopen_path(path.to_str().unwrap()).unwrap();
        f.rewind().unwrap();
        f.write_all(b"after!").unwrap();
        drop(f);
        assert_eq!(std::fs::read(&path).unwrap(), b"after!");
    }
}
