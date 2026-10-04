//! Helpers for raw block devices, which want sector-sized, sector-aligned I/O.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::io::{self, Read};
use std::ops::{Deref, DerefMut};

/// Alignment for I/O buffers. 4096 covers 512e and 4Kn drives, and satisfies
/// O_DIRECT on Linux and FILE_FLAG_NO_BUFFERING on Windows.
pub const ALIGN: usize = 4096;

/// A zeroed heap buffer aligned to [`ALIGN`].
pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
}

// The buffer owns its allocation exclusively.
unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    pub fn new(len: usize) -> Self {
        assert!(len > 0 && len % ALIGN == 0, "length must be a multiple of {ALIGN}");
        let layout = Layout::from_size_align(len, ALIGN).expect("valid layout");
        let ptr = unsafe { alloc_zeroed(layout) };
        assert!(!ptr.is_null(), "allocation failed");
        Self { ptr, len }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, Layout::from_size_align_unchecked(self.len, ALIGN)) }
    }
}

impl Deref for AlignedBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl DerefMut for AlignedBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

/// Reads until `buf` is full or the source ends. Returns the number of bytes read.
/// Decompressors return short reads all the time; this hides that.
pub fn read_full(src: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match src.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Rounds `n` up to a multiple of `sector`.
pub fn round_up(n: u64, sector: u64) -> u64 {
    n.div_ceil(sector) * sector
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned() {
        let b = AlignedBuf::new(ALIGN * 3);
        assert_eq!(b.as_ptr() as usize % ALIGN, 0);
        assert_eq!(b.len(), ALIGN * 3);
        assert!(b.iter().all(|&x| x == 0));
    }

    #[test]
    fn read_full_handles_short_reads() {
        struct Trickle(Vec<u8>);
        impl Read for Trickle {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() || buf.is_empty() {
                    return Ok(0);
                }
                buf[0] = self.0.remove(0);
                Ok(1)
            }
        }
        let mut src = Trickle(vec![1, 2, 3, 4, 5]);
        let mut buf = [0u8; 4];
        assert_eq!(read_full(&mut src, &mut buf).unwrap(), 4);
        assert_eq!(buf, [1, 2, 3, 4]);
        assert_eq!(read_full(&mut src, &mut buf).unwrap(), 1);
    }

    #[test]
    fn rounding() {
        assert_eq!(round_up(0, 512), 0);
        assert_eq!(round_up(1, 512), 512);
        assert_eq!(round_up(512, 512), 512);
        assert_eq!(round_up(513, 512), 1024);
    }
}
