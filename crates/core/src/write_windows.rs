//! Writing a Windows installer ISO as a FAT32 stick that UEFI firmware boots.
//!
//! Windows ISOs aren't hybrid images, so they can't be written raw. Instead:
//! partition the drive, format the partition FAT32, and copy the ISO's files
//! across. `sources/install.wim` is usually over FAT32's 4 GiB file limit, so
//! it's split into `install.swm`, `install2.swm`, ... which Windows Setup
//! reassembles by itself.
//!
//! Like the raw path, the start of the drive is zeroed first and the partition
//! table written last, so the OS doesn't mount the volume halfway through.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use sha2::{Digest, Sha256};

use crate::blockcache::BlockCache;
use crate::blockio::AlignedBuf;
use crate::ipc::PartitionScheme;
use crate::partition::{self, Layout};
use crate::progress::{Phase, Progress, Tracker};
use crate::udf::{Entry, Udf};
use crate::write_raw::Target;
use crate::{check_cancel, Error, Result};

/// Largest file FAT32 can hold.
pub const FAT32_MAX_FILE: u64 = u32::MAX as u64;
/// Size of each `.swm` part, comfortably under the FAT32 limit.
pub const SPLIT_PART_MIB: u32 = 3800;

/// Splits a WIM into `.swm` parts. The helper implements this with wimlib.
pub trait WimSplitter {
    /// Splits `wim` into parts of at most `part_mib` MiB inside `dest`, and
    /// returns them in order (`install.swm`, `install2.swm`, ...).
    fn split(&self, wim: &Path, dest: &Path, part_mib: u32) -> io::Result<Vec<PathBuf>>;
}

/// Settings for one Windows write.
pub struct WindowsJob<'a> {
    pub scheme: PartitionScheme,
    /// Where install.wim is extracted and split (needs ~2× its size free).
    pub temp_dir: &'a Path,
    /// Free space in `temp_dir`, if known.
    pub temp_free: Option<u64>,
    pub splitter: &'a dyn WimSplitter,
    /// install.wim bigger than this gets split. [`FAT32_MAX_FILE`] outside tests.
    pub split_above: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenFile {
    pub path: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct WindowsOutcome {
    pub layout: Layout,
    pub files: Vec<WrittenFile>,
    pub bytes: u64,
}

fn io_err(e: impl std::fmt::Display) -> Error {
    Error::Io(io::Error::other(e.to_string()))
}

/// An 11-character FAT label from the ISO's volume name.
pub fn fat_label(name: &str) -> [u8; 11] {
    let mut label = [b' '; 11];
    let cleaned = name
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    for (slot, c) in label.iter_mut().zip(cleaned) {
        *slot = c as u8;
    }
    if label == [b' '; 11] {
        label.copy_from_slice(b"WINSETUP   ");
    }
    label
}

/// Copies `src` to `dst`, hashing and reporting progress.
fn copy_hashed(
    src: &mut impl Read,
    dst: &mut impl Write,
    tracker: &mut Tracker,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(u64, [u8; 32])> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        check_cancel(cancel)?;
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        dst.write_all(&buf[..n])?;
        total += n as u64;
        if let Some(p) = tracker.advance(n as u64) {
            on_progress(p);
        }
    }
    Ok((total, hasher.finalize().into()))
}

fn is_install_image(path: &str) -> bool {
    path.eq_ignore_ascii_case("sources/install.wim")
}

pub fn write_windows<R: Read + Seek>(
    iso: R,
    dev: &mut impl Target,
    drive_size: u64,
    sector: u64,
    job: &WindowsJob,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<WindowsOutcome> {
    let mut udf = Udf::open(iso)?;
    let label = fat_label(udf.volume_label());
    let entries = udf.walk()?;

    // Size checks up front, before anything is written.
    let mut total = 0u64;
    let mut split = None;
    for (path, e) in entries.iter().filter(|(_, e)| !e.is_dir) {
        total += e.size;
        if is_install_image(path) && e.size > job.split_above {
            split = Some(e.clone());
            total += e.size; // extracted to temp first, then copied as parts
        } else if e.size > FAT32_MAX_FILE {
            return Err(Error::Unsupported(format!(
                "{path} is {} bytes, too big for FAT32 and can't be split",
                e.size
            )));
        }
    }
    let layout = partition::plan(drive_size, sector, job.scheme).ok_or(Error::TooLarge {
        image: total,
        drive: drive_size,
    })?;
    // FAT32 itself takes a little space; leave a margin.
    if total + total / 50 + 64 * partition::MIB > layout.len {
        return Err(Error::TooLarge {
            image: total,
            drive: layout.len,
        });
    }
    if let Some(wim) = &split {
        let free = job.temp_free.unwrap_or(u64::MAX);
        if free < wim.size * 2 + 256 * partition::MIB {
            return Err(Error::Unsupported(format!(
                "Splitting install.wim needs about {} GB of free space in {}, but only {} GB is free.",
                (wim.size * 2) / 1_000_000_000 + 1,
                job.temp_dir.display(),
                free / 1_000_000_000
            )));
        }
    }

    let mut tracker = Tracker::new(Phase::Write, Some(total));
    let mut files = Vec::new();

    // 1. Zero the area before the partition, which holds the old partition table.
    let zeros = AlignedBuf::new(partition::MIB as usize);
    dev.seek(SeekFrom::Start(0))?;
    dev.write_all(&zeros[..layout.start as usize])?;

    // 2. Format and fill the partition.
    {
        let mut cache = BlockCache::new(dev, layout.start, layout.len, sector);
        fatfs::format_volume(
            &mut cache,
            FormatVolumeOptions::new()
                .fat_type(FatType::Fat32)
                .bytes_per_sector(sector as u16)
                .volume_label(label),
        )?;
        {
            let fs = FileSystem::new(&mut cache, FsOptions::new())?;
            let root = fs.root_dir();
            for (path, e) in &entries {
                check_cancel(cancel)?;
                if e.is_dir {
                    root.create_dir(path)?;
                    continue;
                }
                if split.as_ref() == Some(e) {
                    let (_work_dir, parts) =
                        split_wim(&mut udf, e, job, &mut tracker, cancel, on_progress)?;
                    let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
                    for part in parts {
                        let name = part
                            .file_name()
                            .ok_or_else(|| io_err("split produced a nameless file"))?
                            .to_string_lossy()
                            .into_owned();
                        let dest = format!("{dir}/{name}");
                        let mut src = File::open(&part)?;
                        let mut f = root.create_file(&dest)?;
                        f.truncate()?;
                        let (size, sha256) =
                            copy_hashed(&mut src, &mut f, &mut tracker, cancel, on_progress)?;
                        drop(src);
                        let _ = std::fs::remove_file(&part);
                        files.push(WrittenFile {
                            path: dest,
                            size,
                            sha256,
                        });
                    }
                    continue;
                }
                let mut f = root.create_file(path)?;
                f.truncate()?;
                let mut src = udf.open_file(e)?;
                let (size, sha256) =
                    copy_hashed(&mut src, &mut f, &mut tracker, cancel, on_progress)?;
                files.push(WrittenFile {
                    path: path.clone(),
                    size,
                    sha256,
                });
            }
            drop(root);
            fs.unmount()?;
        }
        cache.flush_all()?;
    }

    // 3. The partition table, last.
    check_cancel(cancel)?;
    let mut random = [0u8; 36];
    getrandom::fill(&mut random).map_err(io_err)?;
    for (offset, bytes) in partition::table(layout, drive_size, sector, job.scheme, random) {
        dev.seek(SeekFrom::Start(offset))?;
        dev.write_all(&bytes)?;
    }
    dev.sync()?;

    on_progress(tracker.snapshot());
    Ok(WindowsOutcome {
        layout,
        bytes: files.iter().map(|f| f.size).sum(),
        files,
    })
}

/// Removes a directory when dropped.
struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Extracts install.wim to the temp dir and splits it there. The returned
/// guard deletes the work directory once the caller has copied the parts.
fn split_wim<R: Read + Seek>(
    udf: &mut Udf<R>,
    wim: &Entry,
    job: &WindowsJob,
    tracker: &mut Tracker,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(DirGuard, Vec<PathBuf>)> {
    let work = DirGuard(
        job.temp_dir
            .join(format!("inphiso-split-{}", std::process::id())),
    );
    std::fs::create_dir_all(&work.0)?;
    let extracted = work.0.join("install.wim");
    {
        let mut out = File::create(&extracted)?;
        let mut src = udf.open_file(wim)?;
        copy_hashed(&mut src, &mut out, tracker, cancel, on_progress)?;
        out.sync_all()?;
    }
    let parts_dir = work.0.join("parts");
    std::fs::create_dir_all(&parts_dir)?;
    let parts = job
        .splitter
        .split(&extracted, &parts_dir, SPLIT_PART_MIB)
        .map_err(|e| Error::Unsupported(format!("couldn't split install.wim: {e}")))?;
    let _ = std::fs::remove_file(&extracted);
    if parts.is_empty() {
        return Err(Error::Unsupported(
            "splitting install.wim produced no parts".into(),
        ));
    }
    Ok((work, parts))
}

/// Reads every written file back through a fresh FAT mount and checks its hash.
pub fn verify_windows(
    dev: &mut impl Target,
    sector: u64,
    outcome: &WindowsOutcome,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let mut tracker = Tracker::new(Phase::Verify, Some(outcome.bytes));
    let mut cache = BlockCache::new(dev, outcome.layout.start, outcome.layout.len, sector);
    let fs = FileSystem::new(&mut cache, FsOptions::new())?;
    let root = fs.root_dir();
    for file in &outcome.files {
        let mut f = root.open_file(&file.path)?;
        let (size, sha256) =
            copy_hashed(&mut f, &mut io::sink(), &mut tracker, cancel, on_progress)?;
        if size != file.size || sha256 != file.sha256 {
            return Err(Error::VerifyMismatch);
        }
    }
    on_progress(tracker.snapshot());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_udf;
    use std::io::Cursor;

    #[test]
    fn labels() {
        assert_eq!(&fat_label("CCCOMA_X64FRE_EN-US_DV9"), b"CCCOMA_X64F");
        assert_eq!(&fat_label("win 11"), b"WIN11      ");
        assert_eq!(&fat_label(""), b"WINSETUP   ");
    }

    /// Splits by plain chunking: not a real WIM split, but exercises the plumbing.
    struct ChunkSplitter(usize);
    impl WimSplitter for ChunkSplitter {
        fn split(&self, wim: &Path, dest: &Path, _part_mib: u32) -> io::Result<Vec<PathBuf>> {
            let data = std::fs::read(wim)?;
            let mut out = Vec::new();
            for (i, chunk) in data.chunks(self.0).enumerate() {
                let name = if i == 0 {
                    "install.swm".to_string()
                } else {
                    format!("install{}.swm", i + 1)
                };
                let p = dest.join(name);
                std::fs::write(&p, chunk)?;
                out.push(p);
            }
            Ok(out)
        }
    }

    fn windows_iso(dir: &Path) -> Option<PathBuf> {
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("sources")).unwrap();
        std::fs::create_dir_all(src.join("efi/boot")).unwrap();
        std::fs::write(src.join("bootmgr"), b"boot manager").unwrap();
        std::fs::write(src.join("efi/boot/bootx64.efi"), vec![0xEF; 70_000]).unwrap();
        let wim: Vec<u8> = (0..2_500_000u32).map(|i| (i % 241) as u8).collect();
        std::fs::write(src.join("sources/install.wim"), wim).unwrap();
        let iso = dir.join("win.iso");
        make_udf(&src, &iso).then_some(iso)
    }

    fn job<'a>(
        scheme: PartitionScheme,
        dir: &'a Path,
        splitter: &'a dyn WimSplitter,
        split_above: u64,
    ) -> WindowsJob<'a> {
        WindowsJob {
            scheme,
            temp_dir: dir,
            temp_free: None,
            splitter,
            split_above,
        }
    }

    fn flash(scheme: PartitionScheme, split_above: u64) {
        let dir = tempfile::tempdir().unwrap();
        let Some(iso) = windows_iso(dir.path()) else {
            eprintln!("skipping: no UDF image tool");
            return;
        };
        let drive_size = 256 * partition::MIB;
        let mut dev = Cursor::new(vec![0xAAu8; drive_size as usize]);
        let no = AtomicBool::new(false);
        let splitter = ChunkSplitter(1 << 20);
        let out = write_windows(
            File::open(&iso).unwrap(),
            &mut dev,
            drive_size,
            512,
            &job(scheme, dir.path(), &splitter, split_above),
            &no,
            &mut |_| {},
        )
        .unwrap();
        let paths: Vec<&str> = out.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"bootmgr"));
        assert!(paths.contains(&"efi/boot/bootx64.efi"));
        let split = split_above < 2_500_000;
        if split {
            for p in [
                "sources/install.swm",
                "sources/install2.swm",
                "sources/install3.swm",
            ] {
                assert!(paths.contains(&p), "{p} missing from {paths:?}");
            }
            assert!(!paths.contains(&"sources/install.wim"));
            // The work directory is cleaned up.
            assert!(!dir
                .path()
                .join(format!("inphiso-split-{}", std::process::id()))
                .exists());
        } else {
            assert!(paths.contains(&"sources/install.wim"));
        }

        verify_windows(&mut dev, 512, &out, &no, &mut |_| {}).unwrap();

        // The partition table is there and points at a FAT32 volume.
        let disk = dev.get_ref();
        assert_eq!(&disk[510..512], &[0x55, 0xAA]);
        let start = out.layout.start as usize;
        assert_eq!(&disk[start + 82..start + 90], b"FAT32   ");

        // Corrupt a byte inside a file: verification notices.
        let mut cache = BlockCache::new(&mut dev, out.layout.start, out.layout.len, 512);
        let fs = FileSystem::new(&mut cache, FsOptions::new()).unwrap();
        let mut f = fs.root_dir().open_file("bootmgr").unwrap();
        f.seek(SeekFrom::Start(3)).unwrap();
        f.write_all(b"X").unwrap();
        drop(f);
        fs.unmount().unwrap();
        cache.flush_all().unwrap();
        assert!(matches!(
            verify_windows(&mut dev, 512, &out, &no, &mut |_| {}),
            Err(Error::VerifyMismatch)
        ));
    }

    #[test]
    fn writes_a_bootable_layout_mbr() {
        flash(PartitionScheme::Mbr, FAT32_MAX_FILE);
    }

    #[test]
    fn writes_a_bootable_layout_gpt() {
        flash(PartitionScheme::Gpt, FAT32_MAX_FILE);
    }

    #[test]
    fn splits_install_wim() {
        flash(PartitionScheme::Mbr, 1 << 20);
    }

    #[test]
    fn rejects_drives_that_are_too_small() {
        let dir = tempfile::tempdir().unwrap();
        let Some(iso) = windows_iso(dir.path()) else {
            return;
        };
        let size = 66 * partition::MIB;
        let mut dev = Cursor::new(vec![0u8; size as usize]);
        let splitter = ChunkSplitter(1 << 20);
        let res = write_windows(
            File::open(&iso).unwrap(),
            &mut dev,
            size,
            512,
            &job(PartitionScheme::Mbr, dir.path(), &splitter, FAT32_MAX_FILE),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert!(matches!(res, Err(Error::TooLarge { .. })), "{res:?}");
    }

    /// Writes a stick image to `$INPHISO_DUMP` so another OS can mount it:
    /// `INPHISO_DUMP=/tmp/stick.img cargo test -p inphiso-core dump -- --ignored`
    #[test]
    #[ignore]
    fn dump_stick_image() {
        let Ok(out_path) = std::env::var("INPHISO_DUMP") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let iso = windows_iso(dir.path()).expect("UDF tool");
        let size = 256 * partition::MIB;
        let mut dev = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&out_path)
            .unwrap();
        dev.set_len(size).unwrap();
        let splitter = ChunkSplitter(1 << 20);
        let scheme = match std::env::var("INPHISO_SCHEME").as_deref() {
            Ok("gpt") => PartitionScheme::Gpt,
            _ => PartitionScheme::Mbr,
        };
        write_windows(
            File::open(&iso).unwrap(),
            &mut dev,
            size,
            512,
            &job(scheme, dir.path(), &splitter, FAT32_MAX_FILE),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
    }
}
