//! `inphiso-helper` is the only part of inphiso that runs elevated. The app
//! launches it through an admin prompt / pkexec / UAC, it connects back over a
//! local socket, runs exactly one flash job, and exits.

// No console window flashing up behind the UAC prompt on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use inphiso_core::ipc::{recv, send, AppMsg, HelperMsg, Job, Mode};
use inphiso_core::write_raw::{verify, write_image, Target};
use inphiso_platform::Device;
use interprocess::local_socket::traits::Stream as _;

const USAGE: &str = "usage: inphiso-helper --socket <name> --token <token>";

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
    match run(&socket, token) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
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
        Some(AppMsg::Cancel) | None => return Ok(()),
    };

    // Watch for Cancel. If the app goes away mid-flash, stop too: nobody is
    // left to see the result.
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            loop {
                match recv::<AppMsg>(&mut rx) {
                    Ok(Some(AppMsg::Cancel)) | Ok(None) | Err(_) => break,
                    Ok(Some(AppMsg::Start { .. })) => continue,
                }
            }
            cancel.store(true, Ordering::Relaxed);
        });
    }

    let started = Instant::now();
    let result = flash(&job, &cancel, &mut |progress| {
        let _ = send(&mut tx, &HelperMsg::Progress { progress });
    });
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

/// Runs the job. Returns whether the result was verified.
fn flash(
    job: &Job,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(inphiso_core::progress::Progress),
) -> Result<bool> {
    if let Mode::Windows { .. } = job.mode {
        bail!("Writing Windows installer images isn't supported yet.");
    }

    let meta = std::fs::metadata(&job.image)
        .with_context(|| format!("couldn't open {}", job.image.display()))?;
    if !meta.is_file() {
        bail!("{} isn't a regular file", job.image.display());
    }
    // Decompresses on the fly; the size is unknown for gzip/bzip2/zstd.
    let (image, _, image_size) = inphiso_core::image::open(&job.image)
        .with_context(|| format!("couldn't read {}", job.image.display()))?;

    let device = inphiso_platform::open_for_writing(&job.device, job.device_size)?;
    let (size, sector) = (device.size(), device.sector());
    let mut drive = Drive(device);

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
    drive.0.finish().context("finishing the write")?;
    Ok(job.verify)
}
