//! Windows drive listing through `\\.\PhysicalDriveN` and the storage IOCTLs.
//!
//! Querying a disk's properties and geometry only needs a handle opened with
//! no access rights, so this works without elevation.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::size_of;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    BusTypeMmc, BusTypeNvme, BusTypeSas, BusTypeSata, BusTypeScsi, BusTypeSd, BusTypeUsb,
    CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetVolumeInformationW,
    GetVolumePathNamesForVolumeNameW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING, STORAGE_BUS_TYPE,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, DISK_EXTENT, DISK_GEOMETRY_EX,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_PROPERTY_QUERY, VOLUME_DISK_EXTENTS,
};
use windows::Win32::System::IO::DeviceIoControl;

use crate::{Drive, Result};

/// Highest `PhysicalDriveN` probed. Disk numbers can have gaps after hot-unplugs.
const MAX_DISKS: u32 = 64;

/// A handle closed on drop.
pub struct OwnedHandle(pub HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Opens a device path for IOCTLs with the given access mask.
pub fn open_device(path: &str, access: u32) -> windows::core::Result<OwnedHandle> {
    let h = unsafe {
        CreateFileW(
            &HSTRING::from(path),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )?
    };
    Ok(OwnedHandle(h))
}

/// Runs an IOCTL with no input into an 8-byte-aligned buffer of `len` bytes.
fn ioctl_out(h: &OwnedHandle, code: u32, input: Option<&[u8]>, len: usize) -> Option<Vec<u64>> {
    let mut buf = vec![0u64; len.div_ceil(8)];
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            h.0,
            code,
            input.map(|i| i.as_ptr() as *const c_void),
            input.map_or(0, |i| i.len() as u32),
            Some(buf.as_mut_ptr() as *mut c_void),
            (buf.len() * 8) as u32,
            Some(&mut returned),
            None,
        )
        .ok()?;
    }
    Some(buf)
}

/// Reads a NUL-terminated ASCII string at `offset` within a descriptor buffer.
fn ascii_at(bytes: &[u8], offset: u32) -> Option<String> {
    let start = offset as usize;
    if start == 0 || start >= bytes.len() {
        return None;
    }
    let end = bytes[start..]
        .iter()
        .position(|&b| b == 0)
        .map_or(bytes.len(), |p| start + p);
    let s = String::from_utf8_lossy(&bytes[start..end])
        .trim()
        .to_string();
    (!s.is_empty()).then_some(s)
}

struct DiskProps {
    name: String,
    bus: STORAGE_BUS_TYPE,
    removable_media: bool,
}

fn query_props(h: &OwnedHandle) -> Option<DiskProps> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let input = unsafe {
        std::slice::from_raw_parts(
            &query as *const _ as *const u8,
            size_of::<STORAGE_PROPERTY_QUERY>(),
        )
    };
    let buf = ioctl_out(h, IOCTL_STORAGE_QUERY_PROPERTY, Some(input), 1024)?;
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 8) };
    let desc = unsafe { &*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    let vendor = ascii_at(bytes, desc.VendorIdOffset);
    let product = ascii_at(bytes, desc.ProductIdOffset);
    Some(DiskProps {
        name: crate::join_name(&[vendor.as_deref(), product.as_deref()]),
        bus: desc.BusType,
        removable_media: desc.RemovableMedia,
    })
}

fn disk_size(h: &OwnedHandle) -> Option<u64> {
    let buf = ioctl_out(h, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, None, 256)?;
    let geo = unsafe { &*(buf.as_ptr() as *const DISK_GEOMETRY_EX) };
    u64::try_from(geo.DiskSize).ok()
}

fn bus_label(bus: STORAGE_BUS_TYPE) -> Option<String> {
    let label = match bus {
        b if b == BusTypeUsb => "USB",
        b if b == BusTypeSd || b == BusTypeMmc => "SD card",
        b if b == BusTypeNvme => "NVMe",
        b if b == BusTypeSata => "SATA",
        b if b == BusTypeSas || b == BusTypeScsi => "SCSI",
        _ => return None,
    };
    Some(label.into())
}

struct VolumeInfo {
    disks: Vec<u32>,
    /// "C:\", "E:\" ...
    paths: Vec<String>,
    label: Option<String>,
}

fn utf16_until_nul(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn volume_info(guid_path: &str) -> VolumeInfo {
    // "\\?\Volume{...}\" names the root; the device itself is without the trailing slash.
    let device = guid_path.trim_end_matches('\\');
    let mut disks = Vec::new();
    if let Ok(h) = open_device(device, 0) {
        let len = size_of::<VOLUME_DISK_EXTENTS>() + 31 * size_of::<DISK_EXTENT>();
        if let Some(buf) = ioctl_out(&h, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, None, len) {
            let ext = unsafe { &*(buf.as_ptr() as *const VOLUME_DISK_EXTENTS) };
            let n = (ext.NumberOfDiskExtents as usize).min(32);
            let first = ext.Extents.as_ptr();
            for i in 0..n {
                disks.push(unsafe { (*first.add(i)).DiskNumber });
            }
        }
    }

    let wide = HSTRING::from(guid_path);
    let mut names = vec![0u16; 1024];
    let mut needed = 0u32;
    let paths = unsafe { GetVolumePathNamesForVolumeNameW(&wide, Some(&mut names), &mut needed) }
        .map(|_| {
            names
                .split(|&c| c == 0)
                .take_while(|s| !s.is_empty())
                .map(String::from_utf16_lossy)
                .collect()
        })
        .unwrap_or_default();

    let mut label_buf = vec![0u16; 261];
    let label = unsafe {
        GetVolumeInformationW(
            PCWSTR(wide.as_ptr()),
            Some(&mut label_buf),
            None,
            None,
            None,
            None,
        )
    }
    .ok()
    .map(|_| utf16_until_nul(&label_buf))
    .filter(|l| !l.is_empty());

    VolumeInfo {
        disks,
        paths,
        label,
    }
}

fn all_volumes() -> Vec<VolumeInfo> {
    let mut out = Vec::new();
    let mut buf = vec![0u16; 64];
    let Ok(find) = (unsafe { FindFirstVolumeW(&mut buf) }) else {
        return out;
    };
    loop {
        out.push(volume_info(&utf16_until_nul(&buf)));
        buf.fill(0);
        if unsafe { FindNextVolumeW(find, &mut buf) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindVolumeClose(find);
    }
    out
}

pub fn list() -> Result<Vec<Drive>> {
    let system_drive = std::env::var("SystemDrive")
        .unwrap_or_else(|_| "C:".into())
        .to_ascii_uppercase();
    let system_root = format!("{}\\", system_drive.trim_end_matches('\\'));

    let volumes = all_volumes();
    let mut system_disks = HashSet::new();
    let mut names_by_disk: HashMap<u32, Vec<String>> = HashMap::new();
    for v in &volumes {
        let is_system = v.paths.iter().any(|p| p.eq_ignore_ascii_case(&system_root));
        let display = v.label.clone().or_else(|| {
            v.paths
                .first()
                .map(|p| p.trim_end_matches('\\').to_string())
        });
        for &d in &v.disks {
            if is_system {
                system_disks.insert(d);
            }
            if let Some(name) = &display {
                names_by_disk.entry(d).or_default().push(name.clone());
            }
        }
    }

    let mut drives = Vec::new();
    for n in 0..MAX_DISKS {
        if system_disks.contains(&n) {
            continue;
        }
        let id = format!(r"\\.\PhysicalDrive{n}");
        let Ok(h) = open_device(&id, 0) else { continue };
        let Some(props) = query_props(&h) else {
            continue;
        };
        let Some(size) = disk_size(&h).filter(|&s| s > 0) else {
            continue; // empty card reader slot
        };
        let removable = props.removable_media
            || props.bus == BusTypeUsb
            || props.bus == BusTypeSd
            || props.bus == BusTypeMmc;
        drives.push(Drive {
            id,
            name: if props.name.is_empty() {
                "Unnamed drive".into()
            } else {
                props.name
            },
            size,
            bus: bus_label(props.bus),
            removable,
            volumes: names_by_disk.remove(&n).unwrap_or_default(),
        });
    }
    Ok(drives)
}
