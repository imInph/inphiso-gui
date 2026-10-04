//! Opening a whole disk for raw writing, and ejecting it afterwards.
//!
//! [`open_for_writing`] re-checks the target itself rather than trusting the
//! caller: it must still be listed (the system disk never is) and still have
//! the size the user was shown.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::{Error, Result};

#[cfg(target_os = "linux")]
#[path = "device/linux.rs"]
mod os;
#[cfg(target_os = "macos")]
#[path = "device/macos.rs"]
mod os;
#[cfg(windows)]
#[path = "device/windows.rs"]
mod os;

/// Set to `1` to allow a regular file as the "device". Only honoured in debug
/// builds (tests and development); release builds ignore it.
pub const ALLOW_FILE_ENV: &str = "INPHISO_DEV_ALLOW_FILE";

/// An open whole disk (or test file), ready for sector-aligned I/O.
pub struct Device {
    file: File,
    size: u64,
    sector: u64,
    #[allow(dead_code)]
    guard: os::Guard,
}

impl Device {
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Logical sector size; all I/O offsets and lengths must be multiples of it.
    pub fn sector(&self) -> u64 {
        self.sector
    }

    pub fn sync(&mut self) -> io::Result<()> {
        self.file.sync_all()
    }

    /// Flushes and asks the OS to re-read the new partition table.
    pub fn finish(self) -> io::Result<()> {
        self.file.sync_all()?;
        os::after_write(&self.file);
        Ok(())
    }
}

impl Read for Device {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Write for Device {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Device {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

/// Checks `id` is still a listed (never the system) drive with the size the user saw.
fn validate(id: &str, expected_size: u64) -> Result<crate::Drive> {
    let listed = crate::list_drives(true)?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| {
            Error::Refused(format!(
                "{id} isn't a drive inphiso can write to. Is it still plugged in?"
            ))
        })?;
    if listed.size != expected_size {
        return Err(Error::Refused(format!(
            "{id} has changed size ({} → {} bytes). Pick the drive again.",
            expected_size, listed.size
        )));
    }
    Ok(listed)
}

/// Gets permission to write `id` from the app, before the helper starts.
///
/// On macOS this validates the drive, unmounts it and opens its raw device
/// through `authopen`, which shows the system's admin prompt and hands back the
/// open device. (Privacy rules stop a background process from opening a
/// removable disk itself, even as root.) The app passes the device to the helper.
/// Elsewhere it returns `None`: the elevated helper opens the drive itself.
pub fn authorize(id: &str, expected_size: u64) -> Result<Option<File>> {
    #[cfg(target_os = "macos")]
    {
        validate(id, expected_size)?;
        os::unmount(id)?;
        os::authopen(id).map(Some)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (id, expected_size);
        Ok(None)
    }
}

/// Validates `id`, unmounts everything on it and opens it for writing, or uses
/// `handed`, the raw device the app already opened (see [`authorize`]).
pub fn open_for_writing(id: &str, expected_size: u64, handed: Option<File>) -> Result<Device> {
    if cfg!(debug_assertions) && std::env::var(ALLOW_FILE_ENV).as_deref() == Ok("1") {
        if let Ok(meta) = std::fs::metadata(id) {
            if meta.is_file() {
                // `Guard` is a unit struct on Unix but holds volume locks on Windows.
                #[allow(clippy::default_constructed_unit_structs)]
                let guard = os::Guard::default();
                let file = File::options().read(true).write(true).open(id)?;
                return Ok(Device {
                    file,
                    size: meta.len(),
                    sector: 512,
                    guard,
                });
            }
        }
    }

    let listed = validate(id, expected_size)?;
    let (file, sector, guard) = match handed {
        #[allow(clippy::default_constructed_unit_structs)]
        Some(file) => (file, os::sector_size(id), os::Guard::default()),
        None => os::prepare_and_open(id)?,
    };
    Ok(Device {
        file,
        size: listed.size,
        sector,
        guard,
    })
}

/// Unmounts and ejects a drive so it can be unplugged. Doesn't need elevation.
pub fn eject(id: &str) -> Result<()> {
    os::eject(id)
}
