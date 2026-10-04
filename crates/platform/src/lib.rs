//! Everything that differs between Linux, macOS and Windows lives here,
//! behind one set of types so the rest of inphiso doesn't care.

use serde::{Deserialize, Serialize};

pub mod channel;
pub mod device;
pub mod elevate;
pub mod linux;
pub mod macos;
#[cfg(windows)]
pub mod windows;

pub use device::{eject, open_for_writing, Device};

/// A whole physical disk that could be flashed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drive {
    /// OS identifier: `/dev/sdb`, `disk4`, `\\.\PhysicalDrive2`.
    pub id: String,
    /// Human name, e.g. "SanDisk Ultra".
    pub name: String,
    pub size: u64,
    /// "USB", "SD card", "NVMe", ... when the OS tells us.
    pub bus: Option<String>,
    /// True for USB / SD media. Non-removable drives only show with "Show all drives".
    pub removable: bool,
    /// Volume names currently on the disk, shown in the erase confirmation.
    pub volumes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("couldn't run {cmd}: {source}")]
    Spawn {
        cmd: &'static str,
        source: std::io::Error,
    },
    #[error("{cmd} failed: {stderr}")]
    Command { cmd: &'static str, stderr: String },
    #[error("couldn't parse {what}: {detail}")]
    Parse { what: &'static str, detail: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Refused(String),
    #[error("Permission to write to the drive was not given.")]
    AuthCancelled,
    #[error("this isn't supported on this OS")]
    Unsupported,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Lists flashable disks. The disk the OS boots from is never included.
/// Without `show_all`, only removable (USB / SD) disks are returned.
pub fn list_drives(show_all: bool) -> Result<Vec<Drive>> {
    let drives = list_all()?;
    Ok(drives
        .into_iter()
        .filter(|d| show_all || d.removable)
        .collect())
}

#[cfg(target_os = "macos")]
fn list_all() -> Result<Vec<Drive>> {
    macos::list()
}

#[cfg(target_os = "linux")]
fn list_all() -> Result<Vec<Drive>> {
    linux::list()
}

#[cfg(windows)]
fn list_all() -> Result<Vec<Drive>> {
    windows::list()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn list_all() -> Result<Vec<Drive>> {
    Err(Error::Unsupported)
}

/// Runs a command and returns its stdout, turning a non-zero exit into an error.
#[allow(dead_code)] // unused on Windows
pub(crate) fn run(cmd: &'static str, args: &[&str]) -> Result<Vec<u8>> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .output()
        .map_err(|source| Error::Spawn { cmd, source })?;
    if !out.status.success() {
        return Err(Error::Command {
            cmd,
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(out.stdout)
}

/// Collapses whitespace and drops empty parts: " SanDisk  ", "Ultra " → "SanDisk Ultra".
pub(crate) fn join_name(parts: &[Option<&str>]) -> String {
    parts
        .iter()
        .flatten()
        .flat_map(|p| p.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_name_collapses() {
        assert_eq!(
            join_name(&[Some(" SanDisk  "), None, Some("Ultra ")]),
            "SanDisk Ultra"
        );
        assert_eq!(join_name(&[None, Some("  ")]), "");
    }
}
