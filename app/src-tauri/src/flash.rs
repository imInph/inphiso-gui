//! Drives one flash: starts the elevated helper, hands it the job over the
//! local socket and forwards its progress to the frontend.

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Mutex;

use inphiso_core::ipc::{recv, send, AppMsg, HelperMsg, Job, Mode, PartitionScheme};
use inphiso_platform::channel::{listen, random_hex};
use inphiso_platform::elevate::run_elevated;
use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::SendHalf;
use serde::Deserialize;
use tauri::ipc::Channel;

/// The job as the frontend describes it (see `FlashJob` in `app/src/types.ts`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlashJob {
    image_path: PathBuf,
    drive_id: String,
    drive_size: u64,
    mode: String,
    verify: bool,
    partition_scheme: String,
}

impl FlashJob {
    fn into_job(self) -> Result<Job, String> {
        let mode = match self.mode.as_str() {
            "raw" => Mode::Raw,
            "windows" => Mode::Windows {
                scheme: match self.partition_scheme.as_str() {
                    "gpt" => PartitionScheme::Gpt,
                    _ => PartitionScheme::Mbr,
                },
            },
            other => return Err(format!("unknown write mode {other:?}")),
        };
        Ok(Job {
            image: self.image_path,
            device: self.drive_id,
            device_size: self.drive_size,
            mode,
            verify: self.verify,
        })
    }
}

/// Shared between the running flash and `cancel_flash`.
#[derive(Default)]
pub struct FlashState {
    running: AtomicBool,
    cancel: AtomicBool,
    tx: Mutex<Option<SendHalf>>,
}

impl FlashState {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(tx) = self.tx.lock().unwrap().as_mut() {
            let _ = send(tx, &AppMsg::Cancel);
        }
    }
}

/// Appends the end of the helper's log to an error, so failures explain themselves.
fn with_log(message: &str, log: &std::path::Path) -> String {
    let Ok(text) = std::fs::read_to_string(log) else {
        return message.to_string();
    };
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(8)..].join("\n");
    if tail.trim().is_empty() {
        message.to_string()
    } else {
        format!("{message}\n\n{tail}")
    }
}

/// The helper sits next to the app executable (Tauri puts sidecars there).
fn helper_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let name = if cfg!(windows) {
        "inphiso-helper.exe"
    } else {
        "inphiso-helper"
    };
    let path = exe
        .parent()
        .ok_or("can't locate the app directory")?
        .join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "inphiso-helper is missing ({}). Reinstall inphiso.",
            path.display()
        ))
    }
}

/// Runs the whole flash on the calling (blocking) thread. Every outcome,
/// including failures before the helper connects, reaches the UI as an event.
pub fn run(state: &FlashState, job: FlashJob, events: &Channel<HelperMsg>) {
    if state.running.swap(true, Ordering::SeqCst) {
        let _ = events.send(HelperMsg::Error {
            message: "A flash is already running.".into(),
        });
        return;
    }
    state.cancel.store(false, Ordering::SeqCst);
    let outcome = drive(state, job, events);
    *state.tx.lock().unwrap() = None;
    state.running.store(false, Ordering::SeqCst);
    if let Some(msg) = outcome {
        let _ = events.send(msg);
    }
}

/// Returns a final message to send if the helper didn't send one itself.
fn drive(state: &FlashState, job: FlashJob, events: &Channel<HelperMsg>) -> Option<HelperMsg> {
    let fail = |message: String| Some(HelperMsg::Error { message });
    let job = match job.into_job() {
        Ok(j) => j,
        Err(e) => return fail(e),
    };
    // Open the image here, with the user's permissions: on macOS the elevated
    // helper isn't allowed to open files on external volumes or in protected
    // folders, so it gets this open file instead (on Unix).
    let image_file = match std::fs::File::open(&job.image) {
        Ok(f) => f,
        Err(e) => return fail(format!("couldn't open {}: {e}", job.image.display())),
    };
    // macOS: the drive is opened here through authopen (the admin prompt) and
    // handed to the helper, which then doesn't need to run as root.
    let device_file = match inphiso_platform::device::authorize(&job.device, job.device_size) {
        Ok(f) => f,
        Err(inphiso_platform::Error::AuthCancelled) => return Some(HelperMsg::Cancelled),
        Err(e) => return fail(e.to_string()),
    };
    let helper = match helper_path() {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let server = match listen() {
        Ok(s) => s,
        Err(e) => return fail(format!("couldn't open a channel to the writer: {e}")),
    };
    let token = random_hex(16);

    let (done_tx, done_rx) = mpsc::channel();
    {
        let args = vec![
            "--socket".to_string(),
            server.name.clone(),
            "--token".to_string(),
            token.clone(),
            "--log".to_string(),
            server.log_path.to_string_lossy().into_owned(),
        ];
        let elevate = device_file.is_none();
        std::thread::spawn(move || {
            let result = if elevate {
                run_elevated(&helper, &args)
            } else {
                inphiso_platform::elevate::run_direct(&helper, &args)
            };
            let _ = done_tx.send(result);
        });
    }

    // Wait for the helper to connect, while the user deals with the password prompt.
    let waited = server.accept_polling(|| {
        match done_rx.try_recv() {
            Ok(Err(inphiso_platform::Error::AuthCancelled)) => return Some(HelperMsg::Cancelled),
            Ok(Err(e)) => return fail(with_log(&e.to_string(), &server.log_path)),
            Ok(Ok(())) => {
                return fail(with_log(
                    "the writer exited before it connected",
                    &server.log_path,
                ))
            }
            Err(TryRecvError::Disconnected) => return fail("the writer didn't start".into()),
            Err(TryRecvError::Empty) => {}
        }
        // Can't stop the password prompt once it's up, but nothing gets written.
        state
            .cancel
            .load(Ordering::SeqCst)
            .then_some(HelperMsg::Cancelled)
    });
    let conn = match waited {
        Ok(Ok(conn)) => conn,
        Ok(Err(msg)) => return Some(msg),
        Err(e) => return fail(format!("the writer couldn't connect: {e}")),
    };

    #[cfg(unix)]
    let socket_fd = inphiso_platform::channel::raw_fd(&conn);
    let (rx, mut tx) = conn.split();
    let mut rx = BufReader::new(rx);
    match recv::<HelperMsg>(&mut rx) {
        Ok(Some(HelperMsg::Hello { token: t, version }))
            if t == token && version == env!("CARGO_PKG_VERSION") => {}
        Ok(Some(HelperMsg::Hello { version, .. })) if version != env!("CARGO_PKG_VERSION") => {
            return fail(format!(
                "the writer is version {version} but the app is {}. Reinstall inphiso.",
                env!("CARGO_PKG_VERSION")
            ));
        }
        _ => return fail("the writer didn't identify itself".into()),
    }

    #[cfg(unix)]
    {
        use inphiso_platform::channel::send_file;
        if let Err(e) = send_file(socket_fd, Some(&image_file))
            .and_then(|_| send_file(socket_fd, device_file.as_ref()))
        {
            return fail(format!("couldn't hand the image to the writer: {e}"));
        }
    }
    #[cfg(not(unix))]
    drop((image_file, device_file));
    if let Err(e) = send(&mut tx, &AppMsg::Start { job }) {
        return fail(format!("couldn't send the job to the writer: {e}"));
    }
    *state.tx.lock().unwrap() = Some(tx);
    // A cancel that arrived while connecting.
    if state.cancel.load(Ordering::SeqCst) {
        state.cancel();
    }

    loop {
        match recv::<HelperMsg>(&mut rx) {
            Ok(Some(HelperMsg::Hello { .. })) => {}
            Ok(Some(msg)) => {
                let last = !matches!(msg, HelperMsg::Progress { .. });
                let _ = events.send(msg);
                if last {
                    return None;
                }
            }
            Ok(None) | Err(_) => {
                return fail(with_log(
                    "the writer stopped unexpectedly",
                    &server.log_path,
                ))
            }
        }
    }
}
