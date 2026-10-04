//! Downloading an image from an HTTPS link, with resume, before flashing it.
//!
//! A published checksum next to the image (`<url>.sha256`, `SHA256SUMS`, ...)
//! is fetched too and saved as `<file>.sha256`, where the checksum command
//! finds it like any local one.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use inphiso_core::checksum::find_in_sums;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, RANGE};
use reqwest::{StatusCode, Url};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

/// Matches `DownloadProgress` in `app/src/types.ts`.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    bytes: u64,
    total: Option<u64>,
    speed: f64,
}

#[derive(Default)]
pub struct DownloadState {
    cancel: AtomicBool,
}

const CANCELLED: &str = "Download cancelled";

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(concat!("inphiso/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// A safe file name from the server's Content-Disposition or the URL path.
fn file_name(url: &Url, disposition: Option<&str>) -> String {
    let from_header = disposition.and_then(|d| {
        d.split(';')
            .map(str::trim)
            .find_map(|p| p.strip_prefix("filename="))
            .map(|n| n.trim_matches('"').to_string())
    });
    let from_url = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .map(percent_decode);
    let raw = from_header.or(from_url).unwrap_or_default();
    let clean: String = raw
        .chars()
        .filter(|c| {
            !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()
        })
        .collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() {
        "image.iso".into()
    } else {
        clean
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `name.iso` → `name (1).iso` until it doesn't exist.
fn unique(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    if !path.exists() {
        return path;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) => (&name[..i], &name[i..]),
        None => (name, ""),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

/// Tries the usual places a distro puts checksums next to an image.
fn fetch_published_sum(client: &Client, url: &Url, name: &str) -> Option<String> {
    let mut candidates = vec![
        (format!("{url}.sha256"), true),
        (format!("{url}.sha256sum"), true),
    ];
    for shared in ["SHA256SUMS", "SHA256SUMS.txt", "sha256sum.txt"] {
        if let Ok(u) = url.join(shared) {
            candidates.push((u.to_string(), false));
        }
    }
    for (candidate, dedicated) in candidates {
        let Ok(resp) = client
            .get(&candidate)
            .timeout(Duration::from_secs(15))
            .send()
        else {
            continue;
        };
        if !resp.status().is_success() || resp.content_length().is_some_and(|l| l > 1 << 20) {
            continue;
        }
        let mut text = String::new();
        if resp.take(1 << 20).read_to_string(&mut text).is_err() {
            continue;
        }
        if let Some(hash) = find_in_sums(&text, name, dedicated) {
            return Some(hash);
        }
    }
    None
}

fn run(
    url: &str,
    dir: PathBuf,
    cancel: &AtomicBool,
    on_progress: &Channel<DownloadProgress>,
) -> Result<PathBuf, String> {
    let url = Url::parse(url).map_err(|e| format!("That isn't a valid link: {e}"))?;
    if url.scheme() != "https" {
        return Err("Only https:// links are supported.".into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let client = client()?;

    let first = client
        .get(url.clone())
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Couldn't download: {e}"))?;
    let final_url = first.url().clone();
    let name = file_name(
        &final_url,
        first
            .headers()
            .get(CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok()),
    );
    let total = first.content_length();

    // Already downloaded earlier: reuse it.
    let existing = dir.join(&name);
    if let (Ok(meta), Some(t)) = (std::fs::metadata(&existing), total) {
        if meta.len() == t {
            drop(first);
            save_sum(&client, &final_url, &name, &existing);
            return Ok(existing);
        }
    }

    let dest = unique(&dir, &name);
    let part = dest.with_file_name(format!(
        "{}.part",
        dest.file_name().unwrap().to_string_lossy()
    ));
    let resume_from = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

    let (mut resp, mut file, start) = if resume_from > 0 && total.is_some_and(|t| resume_from < t) {
        drop(first);
        let resp = client
            .get(final_url.clone())
            .header(RANGE, format!("bytes={resume_from}-"))
            .send()
            .map_err(|e| format!("Couldn't resume the download: {e}"))?;
        if resp.status() == StatusCode::PARTIAL_CONTENT {
            let f = OpenOptions::new()
                .append(true)
                .open(&part)
                .map_err(|e| e.to_string())?;
            (resp, f, resume_from)
        } else {
            let f = File::create(&part).map_err(|e| e.to_string())?;
            (resp.error_for_status().map_err(|e| e.to_string())?, f, 0)
        }
    } else {
        let f = File::create(&part).map_err(|e| e.to_string())?;
        (first, f, 0)
    };
    let total = total.or_else(|| {
        resp.headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
            .map(|l| l + start)
    });

    let mut buf = vec![0u8; 1 << 20];
    let mut done = start;
    let began = Instant::now();
    let mut last = Instant::now() - Duration::from_secs(1);
    loop {
        if cancel.load(Ordering::Relaxed) {
            // Keep the .part file so a retry resumes.
            return Err(CANCELLED.into());
        }
        let n = resp
            .read(&mut buf)
            .map_err(|e| format!("The download broke off: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("Couldn't save the download: {e}"))?;
        done += n as u64;
        if last.elapsed() >= Duration::from_millis(200) {
            last = Instant::now();
            let secs = began.elapsed().as_secs_f64().max(0.001);
            let _ = on_progress.send(DownloadProgress {
                bytes: done,
                total,
                speed: (done - start) as f64 / secs,
            });
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    if let Some(t) = total {
        if done != t {
            return Err(format!(
                "The download ended early ({done} of {t} bytes). Try again to resume."
            ));
        }
    }
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    save_sum(&client, &final_url, &name, &dest);
    Ok(dest)
}

/// Saves a published checksum as `<file>.sha256` if one can be found.
fn save_sum(client: &Client, url: &Url, name: &str, dest: &Path) {
    if let Some(hash) = fetch_published_sum(client, url, name) {
        let file = dest.file_name().unwrap().to_string_lossy();
        let _ = std::fs::write(
            dest.with_file_name(format!("{file}.sha256")),
            format!("{hash}  {file}\n"),
        );
    }
}

#[tauri::command]
pub async fn download(
    app: AppHandle,
    state: State<'_, Arc<DownloadState>>,
    url: String,
    dir: Option<PathBuf>,
    on_progress: Channel<DownloadProgress>,
) -> Result<PathBuf, String> {
    let dir = match dir {
        Some(d) => d,
        None => app.path().download_dir().map_err(|e| e.to_string())?,
    };
    let state = Arc::clone(&state);
    state.cancel.store(false, Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || run(&url, dir, &state.cancel, &on_progress))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_download(state: State<'_, Arc<DownloadState>>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_files_safely() {
        let u = Url::parse("https://cdimage.ubuntu.com/24.04/ubuntu%2024.04.iso?x=1").unwrap();
        assert_eq!(file_name(&u, None), "ubuntu 24.04.iso");
        assert_eq!(
            file_name(&u, Some(r#"attachment; filename="../../evil.iso""#)),
            "evil.iso"
        );
        let bare = Url::parse("https://example.com/").unwrap();
        assert_eq!(file_name(&bare, None), "image.iso");
    }

    #[test]
    fn unique_names() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique(dir.path(), "a.iso"), dir.path().join("a.iso"));
        std::fs::write(dir.path().join("a.iso"), b"").unwrap();
        assert_eq!(unique(dir.path(), "a.iso"), dir.path().join("a (1).iso"));
        std::fs::write(dir.path().join("os-24.04.iso"), b"").unwrap();
        assert_eq!(
            unique(dir.path(), "os-24.04.iso"),
            dir.path().join("os-24.04 (1).iso")
        );
    }

    /// Hits the network; run with `cargo test -p inphiso-app -- --ignored`.
    #[test]
    #[ignore]
    fn downloads_over_https() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();
        let events = Channel::new(|_| Ok(()));
        let path = run(
            "https://releases.ubuntu.com/24.04/SHA256SUMS",
            dir.path().to_path_buf(),
            &AtomicBool::new(false),
            &events,
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(".iso"), "{text}");
        assert!(run(
            "http://releases.ubuntu.com/24.04/SHA256SUMS",
            dir.path().to_path_buf(),
            &AtomicBool::new(false),
            &events
        )
        .is_err());
    }
}
