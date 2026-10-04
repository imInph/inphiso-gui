//! `inphiso-helper` is the only part of inphiso that runs elevated. The app
//! launches it through an admin prompt / pkexec / UAC, it connects back over a
//! local socket, runs exactly one flash job, and exits.

// No console window flashing up behind the UAC prompt on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use inphiso_core::image::Compression;
use inphiso_core::ipc::{recv, send, AppMsg, HelperMsg, Job, Mode};
use inphiso_core::write_raw::{verify, write_image, Target};
use inphiso_core::write_windows::{
    verify_windows, write_windows, WimSplitter, WindowsJob, FAT32_MAX_FILE,
};
use inphiso_platform::fs::{big_temp_dir, find_tool, free_space};
use inphiso_platform::Device;
use interprocess::local_socket::traits::Stream as _;

const USAGE: &str = "usage: inphiso-helper --socket <name> --token <token> [--log <file>]";

/// Optional log file (`--log`), so a failure can be explained even when the
/// helper's stdout and stderr go nowhere (as under the macOS admin prompt).
static LOG: std::sync::Mutex<Option<File>> = std::sync::Mutex::new(None);
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn log(msg: impl std::fmt::Display) {
    let secs = STARTED.get_or_init(Instant::now).elapsed().as_secs_f32();
    if let Some(f) = LOG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = writeln!(f, "[{secs:7.2}s] {msg}");
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("inphiso-helper {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let (Some(socket), Some(token)) = (value("--socket"), value("--token")) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if let Some(path) = value("--log") {
        if let Ok(f) = File::options().create(true).append(true).open(path) {
            *LOG.lock().unwrap() = Some(f);
        }
    }
    STARTED.get_or_init(Instant::now);
    std::panic::set_hook(Box::new(|info| log(format!("panic: {info}"))));
    log(format!(
        "inphiso-helper {} started",
        env!("CARGO_PKG_VERSION")
    ));
    match run(&socket, token) {
        Ok(()) => {
            log("finished");
            ExitCode::SUCCESS
        }
        Err(e) => {
            log(format!("error: {e:#}"));
            eprintln!("inphiso-helper: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(socket: &str, token: String) -> Result<()> {
    let stream = inphiso_platform::channel::connect(socket).context("connecting to inphiso")?;
    let (rx, mut tx) = stream.split();
    let mut rx = BufReader::new(rx);

    send(
        &mut tx,
        &HelperMsg::Hello {
            token,
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )?;
    let job = match recv::<AppMsg>(&mut rx)? {
        Some(AppMsg::Start { job }) => job,
        Some(AppMsg::Cancel) | None => {
            log("cancelled before starting");
            return Ok(());
        }
    };
    log(format!("job: {job:?}"));

    // Watch for Cancel. If the app goes away mid-flash, stop too: nobody is
    // left to see the result.
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            loop {
                match recv::<AppMsg>(&mut rx) {
                    Ok(Some(AppMsg::Cancel)) => {
                        log("cancel requested");
                        break;
                    }
                    Ok(None) | Err(_) => {
                        log("the app closed the connection; stopping");
                        break;
                    }
                    Ok(Some(AppMsg::Start { .. })) => continue,
                }
            }
            cancel.store(true, Ordering::Relaxed);
        });
    }

    let started = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        flash(&job, &cancel, &mut |progress| {
            let _ = send(&mut tx, &HelperMsg::Progress { progress });
        })
    }))
    .unwrap_or_else(|panic| {
        let what = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic");
        Err(anyhow::anyhow!("the writer crashed: {what}"))
    });
    match &result {
        Ok(_) => log("flash done"),
        Err(e) => log(format!("flash ended: {e:#}")),
    }
    let msg = match result {
        Ok(verified) => HelperMsg::Done {
            elapsed_ms: started.elapsed().as_millis() as u64,
            verified,
        },
        Err(e) if is_cancel(&e) => HelperMsg::Cancelled,
        Err(e) => HelperMsg::Error {
            message: format!("{e:#}"),
        },
    };
    send(&mut tx, &msg)?;
    Ok(())
}

fn is_cancel(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<inphiso_core::Error>(),
        Some(inphiso_core::Error::Cancelled)
    )
}

/// Wraps the platform device so the core pipeline can drive it.
struct Drive(Device);

impl Read for Drive {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Drive {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl Seek for Drive {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl Target for Drive {
    fn sync(&mut self) -> std::io::Result<()> {
        self.0.sync()
    }
}

type OnProgress<'a> = &'a mut dyn FnMut(inphiso_core::progress::Progress);

/// Runs the job. Returns whether the result was verified.
fn flash(job: &Job, cancel: &AtomicBool, on_progress: OnProgress) -> Result<bool> {
    let meta = std::fs::metadata(&job.image)
        .with_context(|| format!("couldn't open {}", job.image.display()))?;
    if !meta.is_file() {
        bail!("{} isn't a regular file", job.image.display());
    }
    // Decompresses on the fly; the size is unknown for gzip/bzip2/zstd.
    let (image, compression, image_size) = inphiso_core::image::open(&job.image)
        .with_context(|| format!("couldn't read {}", job.image.display()))?;
    if let Mode::Windows { .. } = job.mode {
        if compression != Compression::None {
            bail!("Windows installers have to be a plain .iso, not a compressed file.");
        }
    }

    log(format!(
        "image opened: {compression:?}, {} bytes to write",
        image_size.map_or("unknown".into(), |s| s.to_string())
    ));
    let device = inphiso_platform::open_for_writing(&job.device, job.device_size)?;
    let (size, sector) = (device.size(), device.sector());
    log(format!(
        "drive opened: {} bytes, {sector}-byte sectors",
        size
    ));
    let mut drive = Drive(device);

    match job.mode {
        Mode::Raw => {
            let out = write_image(
                image,
                &mut drive,
                image_size,
                size,
                sector,
                cancel,
                on_progress,
            )?;
            if job.verify {
                verify(
                    &mut drive,
                    out.bytes,
                    &out.sha256,
                    sector,
                    cancel,
                    on_progress,
                )?;
            }
        }
        Mode::Windows { scheme } => {
            drop(image);
            let iso = BufReader::with_capacity(1 << 20, File::open(&job.image)?);
            let temp_dir = big_temp_dir();
            let splitter = Wimlib(find_tool("wimlib-imagex"));
            let wjob = WindowsJob {
                scheme,
                temp_free: free_space(&temp_dir).ok(),
                temp_dir: &temp_dir,
                splitter: &splitter,
                split_above: FAT32_MAX_FILE,
            };
            let out = write_windows(iso, &mut drive, size, sector, &wjob, cancel, on_progress)?;
            if job.verify {
                verify_windows(&mut drive, sector, &out, cancel, on_progress)?;
            }
        }
    }
    drive.0.finish().context("finishing the write")?;
    Ok(job.verify)
}

/// Splits install.wim with wimlib's command-line tool, shipped next to the helper.
struct Wimlib(Option<PathBuf>);

impl WimSplitter for Wimlib {
    fn split(&self, wim: &Path, dest: &Path, part_mib: u32) -> std::io::Result<Vec<PathBuf>> {
        let tool = self.0.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "wimlib-imagex is missing, so install.wim can't be split. Reinstall inphiso.",
            )
        })?;
        let mut cmd = Command::new(tool);
        cmd.arg("split")
            .arg(wim)
            .arg(dest.join("install.swm"))
            .arg(part_mib.to_string());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // wimlib-imagex is a console program; don't flash a console window.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd.output()?;
        if !out.status.success() {
            return Err(std::io::Error::other(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        // install.swm, install2.swm, install3.swm, ... in order.
        let mut parts: Vec<(u32, PathBuf)> = std::fs::read_dir(dest)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                let n = name.strip_prefix("install")?.strip_suffix(".swm")?;
                let index = if n.is_empty() { 1 } else { n.parse().ok()? };
                Some((index, e.path()))
            })
            .collect();
        parts.sort();
        Ok(parts.into_iter().map(|(_, p)| p).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses the real wimlib when it's installed (CI installs it on Linux).
    #[test]
    fn splits_a_real_wim_in_order() {
        let Some(tool) = find_tool("wimlib-imagex") else {
            eprintln!("skipping: wimlib-imagex not installed");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        // wimlib never splits one file across parts, so use several ~700 KB files of
        // incompressible data to need several 1 MiB parts.
        let mut seed = 1u64;
        for i in 0..6 {
            let data: Vec<u8> = (0..700_000)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect();
            std::fs::write(src.join(format!("payload{i}.bin")), &data).unwrap();
        }
        let wim = dir.path().join("install.wim");
        let ok = Command::new(&tool)
            .args(["capture"])
            .arg(&src)
            .arg(&wim)
            .args(["--compress=none"])
            .status()
            .unwrap()
            .success();
        assert!(ok, "wimlib capture failed");

        let parts_dir = dir.path().join("parts");
        std::fs::create_dir_all(&parts_dir).unwrap();
        let parts = Wimlib(Some(tool)).split(&wim, &parts_dir, 1).unwrap();
        assert!(parts.len() >= 3, "{parts:?}");
        let names: Vec<String> = parts
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names[0], "install.swm");
        assert_eq!(names[1], "install2.swm");
        assert_eq!(
            names[names.len() - 1],
            format!("install{}.swm", names.len())
        );
    }
}
