//! Small file system queries.

use std::io;
use std::path::{Path, PathBuf};

/// Bytes available to the current user on the volume holding `path`.
#[cfg(unix)]
pub fn free_space(path: &Path) -> io::Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    #[allow(clippy::unnecessary_cast)] // the field types differ between platforms
    Ok(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(windows)]
pub fn free_space(path: &Path) -> io::Result<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(
            &HSTRING::from(path.as_os_str()),
            Some(&mut free),
            None,
            None,
        )
    }
    .map_err(|e| io::Error::from_raw_os_error(e.code().0 & 0xffff))?;
    Ok(free)
}

/// A temp directory on real disk, for multi-gigabyte scratch files.
/// On Linux `/tmp` is often RAM-backed tmpfs, so prefer `/var/tmp`.
pub fn big_temp_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let var_tmp = Path::new("/var/tmp");
        if var_tmp.is_dir() {
            return var_tmp.to_path_buf();
        }
    }
    std::env::temp_dir()
}

/// Finds an executable next to the current one, then on `PATH`.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(&file)));
    if let Some(p) = beside.filter(|p| p.is_file()) {
        return Some(p);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(&file))
            .find(|p| p.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_some_free_space() {
        assert!(free_space(&std::env::temp_dir()).unwrap() > 0);
    }

    #[test]
    fn temp_dir_exists() {
        assert!(big_temp_dir().is_dir());
    }
}
