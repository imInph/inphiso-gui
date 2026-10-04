//! Drives one flash: starts the elevated helper, hands it the job over the
//! local socket and forwards its progress to the frontend.

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Mutex;
use std::time::Duration;

use inphiso_core::ipc::{recv, send, AppMsg, HelperMsg, Job, Mode, PartitionScheme};
use inphiso_platform::channel::{listen, random_hex};
use inphiso_platform::elevate::run_elevated;
use interprocess::local_socket::traits::{Listener as _, Stream as _};
use interprocess::local_socket::{ListenerNonblockingMode, SendHalf};
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
        ];
        std::thread::spawn(move || {
            let _ = done_tx.send(run_elevated(&helper, &args));
        });
    }

    // Wait for the helper to connect, while the user deals with the password prompt.
    if let Err(e) = server
        .listener
        .set_nonblocking(ListenerNonblockingMode::Accept)
    {
        return fail(e.to_string());
    }
    let conn = loop {
        match server.listener.accept() {
            Ok(c) => break c,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return fail(format!("the writer couldn't connect: {e}")),
        }
        match done_rx.try_recv() {
            Ok(Err(inphiso_platform::Error::AuthCancelled)) => return Some(HelperMsg::Cancelled),
            Ok(Err(e)) => return fail(format!("{e}")),
            Ok(Ok(())) => return fail("the writer exited before it connected".into()),
            Err(TryRecvError::Disconnected) => return fail("the writer didn't start".into()),
            Err(TryRecvError::Empty) => {}
        }
        // Can't stop the password prompt once it's up, but nothing gets written.
        if state.cancel.load(Ordering::SeqCst) {
            return Some(HelperMsg::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(50));
    };

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
            Ok(None) | Err(_) => return fail("the writer stopped unexpectedly".into()),
        }
    }
}
