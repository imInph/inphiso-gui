//! Byte-for-byte image writing and read-back verification.
//!
//! The first [`HEAD`] bytes of the drive are zeroed before anything else and
//! the image's own first [`HEAD`] bytes are written last. Until the very end
//! the OS sees no partition table, so it doesn't auto-mount half-written
//! partitions mid-flash (Windows in particular then blocks further writes).

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::AtomicBool;

use sha2::{Digest, Sha256};

use crate::blockio::{read_full, round_up, AlignedBuf, ALIGN};
use crate::progress::{Phase, Progress, Tracker};
use crate::{check_cancel, Error, Result, StepExt};

/// Size of each read/write.
pub const CHUNK: usize = 4 << 20;
/// Region holding MBR/GPT headers, written last. Covers MBR, GPT header and entries.
pub const HEAD: usize = 64 << 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    /// Image bytes written (without sector padding).
    pub bytes: u64,
    pub sha256: [u8; 32],
}

/// Where the image goes: a raw device, or any file in tests.
pub trait Target: Read + Write + Seek {
    /// Make sure everything written is on the medium.
    fn sync(&mut self) -> std::io::Result<()> {
        self.flush()
    }
}

impl Target for std::fs::File {
    fn sync(&mut self) -> std::io::Result<()> {
        self.sync_all()
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]>> Target for std::io::Cursor<T> where std::io::Cursor<T>: Write {}

/// Pads `len` up to whole sectors without running past the end of the drive.
fn padded(len: usize, pos: u64, drive_size: u64, sector: u64) -> usize {
    let want = round_up(len as u64, sector);
    want.min(drive_size.saturating_sub(pos)) as usize
}

/// Writes `src` to `dev`. `total` is the image size if known (used for progress
/// and the up-front size check); `drive_size` is enforced either way.
pub fn write_image(
    mut src: impl Read,
    dev: &mut impl Target,
    total: Option<u64>,
    drive_size: u64,
    sector: u64,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<WriteOutcome> {
    assert!(
        sector > 0 && (ALIGN as u64).is_multiple_of(sector),
        "unsupported sector size {sector}"
    );
    if let Some(t) = total {
        if t > drive_size {
            return Err(Error::TooLarge {
                image: t,
                drive: drive_size,
            });
        }
    }

    let mut hasher = Sha256::new();
    let mut tracker = Tracker::new(Phase::Write, total);

    // 1. Wipe the old partition table so the OS forgets the old layout.
    let mut head = AlignedBuf::new(HEAD);
    let wipe = padded(HEAD, 0, drive_size, sector);
    dev.seek(SeekFrom::Start(0)).step("wiping the drive")?;
    dev.write_all(&head[..wipe]).step("wiping the drive")?;

    // 2. Hold back the image's own head.
    let head_len = read_full(&mut src, &mut head).step("reading the image")?;
    hasher.update(&head[..head_len]);
    tracker.advance(head_len as u64);

    // 3. Everything after the head.
    let mut pos = HEAD as u64;
    let mut buf = AlignedBuf::new(CHUNK);
    if head_len == HEAD {
        loop {
            check_cancel(cancel)?;
            let n = read_full(&mut src, &mut buf).step("reading the image")?;
            if n == 0 {
                break;
            }
            if pos + n as u64 > drive_size {
                return Err(Error::TooLarge {
                    image: pos + n as u64,
                    drive: drive_size,
                });
            }
            hasher.update(&buf[..n]);
            let len = padded(n, pos, drive_size, sector);
            buf[n..len].fill(0);
            dev.write_all(&buf[..len]).step("writing to the drive")?;
            pos += n as u64;
            if let Some(p) = tracker.advance(n as u64) {
                on_progress(p);
            }
            if n < CHUNK {
                break;
            }
        }
        dev.sync().step("flushing the drive")?;
    }

    // 4. Finally the head, which makes the drive's new layout appear all at once.
    check_cancel(cancel)?;
    let len = padded(head_len, 0, drive_size, sector);
    head[head_len..len].fill(0);
    dev.seek(SeekFrom::Start(0))
        .step("writing the partition table")?;
    dev.write_all(&head[..len])
        .step("writing the partition table")?;
    dev.sync().step("flushing the drive")?;

    on_progress(tracker.snapshot());
    Ok(WriteOutcome {
        bytes: tracker.bytes(),
        sha256: hasher.finalize().into(),
    })
}

/// Reads the first `bytes` back from `dev` and checks they hash to `expected`.
pub fn verify(
    dev: &mut impl Target,
    bytes: u64,
    expected: &[u8; 32],
    sector: u64,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut tracker = Tracker::new(Phase::Verify, Some(bytes));
    let mut buf = AlignedBuf::new(CHUNK);
    dev.seek(SeekFrom::Start(0))
        .step("reading the drive back")?;
    let mut done = 0u64;
    while done < bytes {
        check_cancel(cancel)?;
        let want = (bytes - done).min(CHUNK as u64) as usize;
        // Raw devices only read whole sectors; read the padded length, hash the real one.
        let read_len = round_up(want as u64, sector) as usize;
        let n = read_full(dev, &mut buf[..read_len]).step("reading the drive back")?;
        if n < want {
            return Err(Error::ShortRead);
        }
        hasher.update(&buf[..want]);
        done += want as u64;
        if let Some(p) = tracker.advance(want as u64) {
            on_progress(p);
        }
    }
    on_progress(tracker.snapshot());
    let actual: [u8; 32] = hasher.finalize().into();
    if &actual != expected {
        return Err(Error::VerifyMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn image(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn sha(data: &[u8]) -> [u8; 32] {
        Sha256::digest(data).into()
    }

    fn flash(img: &[u8], drive: usize) -> (Result<WriteOutcome>, Vec<u8>) {
        let mut dev = Cursor::new(vec![0xAAu8; drive]);
        let res = write_image(
            img,
            &mut dev,
            Some(img.len() as u64),
            drive as u64,
            512,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        (res, dev.into_inner())
    }

    #[test]
    fn writes_large_image_with_odd_tail() {
        let img = image(CHUNK * 2 + HEAD + 1000);
        let (res, dev) = flash(&img, CHUNK * 4);
        let out = res.unwrap();
        assert_eq!(out.bytes, img.len() as u64);
        assert_eq!(out.sha256, sha(&img));
        assert_eq!(&dev[..img.len()], &img[..]);
        // Tail padded with zeros to the sector boundary, untouched after that.
        let padded_end = round_up(img.len() as u64, 512) as usize;
        assert!(dev[img.len()..padded_end].iter().all(|&b| b == 0));
        assert_eq!(dev[padded_end], 0xAA);
    }

    #[test]
    fn writes_image_smaller_than_head() {
        let img = image(5000);
        let (res, dev) = flash(&img, 1 << 20);
        assert_eq!(res.unwrap().sha256, sha(&img));
        assert_eq!(&dev[..5000], &img[..]);
        // The wipe zeroed the rest of the head region.
        assert!(dev[round_up(5000, 512) as usize..HEAD]
            .iter()
            .all(|&b| b == 0));
    }

    #[test]
    fn image_exactly_drive_sized() {
        let img = image(HEAD * 4);
        let (res, dev) = flash(&img, HEAD * 4);
        res.unwrap();
        assert_eq!(dev, img);
    }

    #[test]
    fn rejects_too_large_up_front() {
        let img = image(HEAD * 4);
        let (res, dev) = flash(&img, HEAD * 2);
        assert!(matches!(res, Err(Error::TooLarge { .. })));
        assert!(dev.iter().all(|&b| b == 0xAA), "nothing written");
    }

    #[test]
    fn rejects_too_large_when_size_unknown() {
        let img = image(HEAD * 4);
        let mut dev = Cursor::new(vec![0u8; HEAD * 2]);
        let res = write_image(
            &img[..],
            &mut dev,
            None,
            (HEAD * 2) as u64,
            512,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert!(matches!(res, Err(Error::TooLarge { .. })));
    }

    #[test]
    fn cancel_stops_before_head_is_written() {
        let img = image(CHUNK * 3);
        let mut dev = Cursor::new(vec![0xAAu8; CHUNK * 4]);
        let cancel = AtomicBool::new(true);
        let res = write_image(
            &img[..],
            &mut dev,
            Some(img.len() as u64),
            (CHUNK * 4) as u64,
            512,
            &cancel,
            &mut |_| {},
        );
        assert!(matches!(res, Err(Error::Cancelled)));
        // The drive has no partition table, rather than a misleading one.
        assert!(dev.into_inner()[..HEAD].iter().all(|&b| b == 0));
    }

    #[test]
    fn verify_round_trip_and_mismatch() {
        let img = image(CHUNK + 777);
        let (res, dev) = flash(&img, CHUNK * 2);
        let out = res.unwrap();
        let mut dev = Cursor::new(dev);
        let no = AtomicBool::new(false);
        verify(&mut dev, out.bytes, &out.sha256, 512, &no, &mut |_| {}).unwrap();

        let mut bad = dev.into_inner();
        bad[CHUNK / 2] ^= 1;
        let res = verify(
            &mut Cursor::new(bad),
            out.bytes,
            &out.sha256,
            512,
            &no,
            &mut |_| {},
        );
        assert!(matches!(res, Err(Error::VerifyMismatch)));
    }

    #[test]
    fn reports_final_progress() {
        let img = image(CHUNK + HEAD);
        let mut events = Vec::new();
        let mut dev = Cursor::new(vec![0u8; CHUNK * 2]);
        write_image(
            &img[..],
            &mut dev,
            Some(img.len() as u64),
            (CHUNK * 2) as u64,
            512,
            &AtomicBool::new(false),
            &mut |p| events.push(p),
        )
        .unwrap();
        let last = events.last().unwrap();
        assert_eq!(last.bytes, img.len() as u64);
        assert_eq!(last.fraction(), Some(1.0));
    }
}
