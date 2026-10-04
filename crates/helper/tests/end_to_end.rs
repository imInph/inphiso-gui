//! Runs the real helper binary over a real local socket, with a plain file
//! standing in for the drive.

use std::io::{BufReader, Write};
use std::path::Path;
use std::process::{Child, Command};

use inphiso_core::ipc::{recv, send, AppMsg, HelperMsg, Job, Mode};
use inphiso_platform::channel::{listen, random_hex};
use interprocess::local_socket::traits::Stream as _;

fn spawn_helper(socket: &str, token: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_inphiso-helper"))
        .args(["--socket", socket, "--token", token])
        .env(inphiso_platform::device::ALLOW_FILE_ENV, "1")
        .spawn()
        .expect("spawn helper")
}

fn write_file(path: &Path, len: usize) -> Vec<u8> {
    let data: Vec<u8> = (0..len).map(|i| (i * 7 % 253) as u8).collect();
    std::fs::File::create(path)
        .unwrap()
        .write_all(&data)
        .unwrap();
    data
}

/// Runs one job and returns every message after Hello.
fn run_job(job_for: impl FnOnce() -> Job, cancel_after_first_progress: bool) -> Vec<HelperMsg> {
    let server = listen().unwrap();
    let token = random_hex(16);
    let mut child = spawn_helper(&server.name, &token);

    // Same polling accept the app uses, so its quirks are covered here too.
    let conn = server.accept_polling(|| None::<()>).unwrap().unwrap();
    #[cfg(unix)]
    let fd = inphiso_platform::channel::raw_fd(&conn);
    let (rx, mut tx) = conn.split();
    let mut rx = BufReader::new(rx);

    match recv::<HelperMsg>(&mut rx).unwrap() {
        Some(HelperMsg::Hello { token: t, .. }) => assert_eq!(t, token),
        other => panic!("expected hello, got {other:?}"),
    }
    let job = job_for();
    // Like the app: hand over the open image when it exists, else let the helper try.
    #[cfg(unix)]
    {
        let file = std::fs::File::open(&job.image).ok();
        inphiso_platform::channel::send_file(fd, file.as_ref()).unwrap();
    }
    send(&mut tx, &AppMsg::Start { job }).unwrap();

    let mut msgs = Vec::new();
    while let Some(msg) = recv::<HelperMsg>(&mut rx).unwrap() {
        let first_progress = matches!(msg, HelperMsg::Progress { .. }) && msgs.is_empty();
        let end = matches!(
            msg,
            HelperMsg::Done { .. } | HelperMsg::Error { .. } | HelperMsg::Cancelled
        );
        msgs.push(msg);
        if first_progress && cancel_after_first_progress {
            send(&mut tx, &AppMsg::Cancel).unwrap();
        }
        if end {
            break;
        }
    }
    assert!(child.wait().unwrap().success());
    msgs
}

#[test]
fn flashes_and_verifies_a_file_target() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("image.img");
    let target = dir.path().join("drive.bin");
    let data = write_file(&image, 9 * 1024 * 1024 + 4096);
    std::fs::File::create(&target)
        .unwrap()
        .set_len(16 * 1024 * 1024)
        .unwrap();

    let msgs = run_job(
        || Job {
            image: image.clone(),
            device: target.to_string_lossy().into(),
            device_size: 16 * 1024 * 1024,
            mode: Mode::Raw,
            verify: true,
        },
        false,
    );

    match msgs.last().unwrap() {
        HelperMsg::Done { verified, .. } => assert!(verified),
        other => panic!("expected done, got {other:?}"),
    }
    let phases: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            HelperMsg::Progress { progress } => Some(progress.phase),
            _ => None,
        })
        .collect();
    assert!(phases.contains(&inphiso_core::progress::Phase::Write));
    assert!(phases.contains(&inphiso_core::progress::Phase::Verify));

    let written = std::fs::read(&target).unwrap();
    assert_eq!(&written[..data.len()], &data[..]);
}

#[test]
fn reports_errors_instead_of_crashing() {
    let dir = tempfile::tempdir().unwrap();
    let msgs = run_job(
        || Job {
            image: dir.path().join("missing.iso"),
            device: dir.path().join("nope").to_string_lossy().into(),
            device_size: 1,
            mode: Mode::Raw,
            verify: false,
        },
        false,
    );
    match msgs.last().unwrap() {
        HelperMsg::Error { message } => assert!(message.contains("missing.iso"), "{message}"),
        other => panic!("expected error, got {other:?}"),
    }
}

#[test]
fn cancels_mid_write() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("big.img");
    let target = dir.path().join("drive.bin");
    write_file(&image, 256 * 1024 * 1024);
    std::fs::File::create(&target)
        .unwrap()
        .set_len(300 * 1024 * 1024)
        .unwrap();

    let msgs = run_job(
        || Job {
            image: image.clone(),
            device: target.to_string_lossy().into(),
            device_size: 300 * 1024 * 1024,
            mode: Mode::Raw,
            verify: true,
        },
        true,
    );
    assert_eq!(msgs.last().unwrap(), &HelperMsg::Cancelled);
}

fn make_udf(src: &Path, out: &Path) -> bool {
    let tries: [(&str, &[&str]); 2] = [
        (
            "hdiutil",
            &["makehybrid", "-udf", "-udf-version", "1.02", "-o"],
        ),
        ("genisoimage", &["-quiet", "-udf", "-o"]),
    ];
    tries.iter().any(|(tool, args)| {
        Command::new(tool)
            .args(*args)
            .arg(out)
            .arg(src)
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

#[test]
fn flashes_a_windows_iso_to_a_file_target() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("sources")).unwrap();
    std::fs::create_dir_all(src.join("efi/boot")).unwrap();
    std::fs::write(src.join("bootmgr"), b"boot").unwrap();
    std::fs::write(src.join("efi/boot/bootx64.efi"), b"efi").unwrap();
    write_file(&src.join("sources/install.wim"), 3 * 1024 * 1024);
    let iso = dir.path().join("win.iso");
    if !make_udf(&src, &iso) {
        eprintln!("skipping: no UDF image tool");
        return;
    }
    let target = dir.path().join("drive.bin");
    let size = 128 * 1024 * 1024;
    std::fs::File::create(&target)
        .unwrap()
        .set_len(size)
        .unwrap();

    let msgs = run_job(
        || Job {
            image: iso.clone(),
            device: target.to_string_lossy().into(),
            device_size: size,
            mode: Mode::Windows {
                scheme: inphiso_core::ipc::PartitionScheme::Gpt,
            },
            verify: true,
        },
        false,
    );
    match msgs.last().unwrap() {
        HelperMsg::Done { verified, .. } => assert!(verified),
        other => panic!("expected done, got {other:?}"),
    }
    let disk = std::fs::read(&target).unwrap();
    assert_eq!(&disk[512..520], b"EFI PART");
}

/// Flashes a real image to a sparse file: `INPHISO_E2E_IMAGE=/path/to.iso cargo test
/// -p inphiso-helper real_image -- --ignored --nocapture`
#[test]
#[ignore]
fn real_image_to_a_file_target() {
    let Ok(image) = std::env::var("INPHISO_E2E_IMAGE") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("drive.bin");
    let size = 16_000_000_000u64;
    std::fs::File::create(&target)
        .unwrap()
        .set_len(size)
        .unwrap();
    let msgs = run_job(
        || Job {
            image: image.clone().into(),
            device: target.to_string_lossy().into(),
            device_size: size,
            mode: Mode::Raw,
            verify: false,
        },
        false,
    );
    let last = msgs.last().unwrap();
    eprintln!("{} messages, last: {last:?}", msgs.len());
    assert!(matches!(last, HelperMsg::Done { .. }), "{last:?}");
}
