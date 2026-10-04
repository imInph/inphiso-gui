use std::fs::File;
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::time::Duration;

use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH};
use windows::Win32::System::Ioctl::{
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_DISK_UPDATE_PROPERTIES,
    IOCTL_STORAGE_EJECT_MEDIA, IOCTL_STORAGE_MEDIA_REMOVAL, PREVENT_MEDIA_REMOVAL,
};

use crate::windows::{
    all_volumes, geometry, ioctl_out, open_device, open_device_with, OwnedHandle,
};
use crate::{Error, Result};

/// Locked volume handles. Windows keeps a volume locked and dismounted only
/// while its handle stays open, so these live as long as the [`Device`](super::Device).
#[derive(Default)]
pub struct Guard {
    _volumes: Vec<OwnedHandle>,
}

fn disk_number(id: &str) -> Result<u32> {
    id.strip_prefix(r"\\.\PhysicalDrive")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Error::Refused(format!("{id} isn't a physical drive path")))
}

/// Locks and dismounts every volume on the disk. Retries the lock a few times
/// because Explorer and antivirus scanners briefly hold volumes open.
fn lock_volumes(disk: u32) -> Result<Vec<OwnedHandle>> {
    let mut locked = Vec::new();
    for v in all_volumes()
        .into_iter()
        .filter(|v| v.disks.contains(&disk))
    {
        let h = open_device(&v.device, (GENERIC_READ | GENERIC_WRITE).0)
            .map_err(|e| Error::Refused(format!("couldn't open volume {}: {e}", v.device)))?;
        let mut ok = false;
        for _ in 0..20 {
            if ioctl_out(h.0, FSCTL_LOCK_VOLUME, None, 8).is_some() {
                ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !ok {
            let name = v.paths.first().cloned().unwrap_or(v.device.clone());
            return Err(Error::Refused(format!(
                "{name} is in use. Close any windows or programs using it and try again."
            )));
        }
        // Dismounting can fail on a volume with no file system; the lock is what matters.
        let _ = ioctl_out(h.0, FSCTL_DISMOUNT_VOLUME, None, 8);
        locked.push(h);
    }
    Ok(locked)
}

pub fn prepare_and_open(id: &str) -> Result<(File, u64, Guard)> {
    let volumes = lock_volumes(disk_number(id)?)?;
    let h = open_device_with(
        id,
        (GENERIC_READ | GENERIC_WRITE).0,
        FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH,
    )
    .map_err(|e| Error::Io(std::io::Error::from_raw_os_error(e.code().0 & 0xffff)))?;
    let sector = geometry(&h).map(|g| g.1).filter(|&s| s > 0).unwrap_or(512);
    // Hand the handle to std::fs::File, which then owns and closes it.
    let raw = h.0 .0 as RawHandle;
    std::mem::forget(h);
    let file = unsafe { File::from_raw_handle(raw) };
    Ok((file, sector, Guard { _volumes: volumes }))
}

pub fn after_write(file: &File) {
    let _ = ioctl_out(
        HANDLE(file.as_raw_handle()),
        IOCTL_DISK_UPDATE_PROPERTIES,
        None,
        8,
    );
}

pub fn eject(id: &str) -> Result<()> {
    let disk = disk_number(id)?;
    let volumes = lock_volumes(disk)?;
    let allow = PREVENT_MEDIA_REMOVAL {
        PreventMediaRemoval: false,
    };
    let allow_bytes = unsafe {
        std::slice::from_raw_parts(
            &allow as *const _ as *const u8,
            std::mem::size_of::<PREVENT_MEDIA_REMOVAL>(),
        )
    };
    for h in &volumes {
        let _ = ioctl_out(h.0, IOCTL_STORAGE_MEDIA_REMOVAL, Some(allow_bytes), 8);
        let _ = ioctl_out(h.0, IOCTL_STORAGE_EJECT_MEDIA, None, 8);
    }
    // A stick with no mounted volume (e.g. a raw Linux image) is ejected through the disk.
    if volumes.is_empty() {
        let h = open_device(id, (GENERIC_READ | GENERIC_WRITE).0)
            .map_err(|e| Error::Refused(format!("couldn't open {id}: {e}")))?;
        ioctl_out(h.0, IOCTL_STORAGE_EJECT_MEDIA, None, 8)
            .ok_or_else(|| Error::Refused(format!("Windows refused to eject {id}")))?;
    }
    Ok(())
}
