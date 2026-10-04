//! Image inspection and checksum commands.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use inphiso_core::checksum::{find_published, normalize, sha256_file};
use inphiso_core::image::ImageInfo;
use serde::Serialize;
use tauri::ipc::Channel;

/// Matches `ChecksumState` in `app/src/types.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ChecksumState {
    Hashing {
        bytes: u64,
        total: u64,
    },
    Match {
        actual: String,
        source: String,
    },
    Mismatch {
        actual: String,
        expected: String,
        source: String,
    },
    Unverified {
        actual: String,
    },
}

/// Bumped on every checksum request; an older run sees the change and stops.
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[tauri::command]
pub async fn inspect_image(path: PathBuf) -> Result<ImageInfo, String> {
    tauri::async_runtime::spawn_blocking(move || inphiso_core::image::inspect(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn checksum(
    path: PathBuf,
    expected: Option<String>,
    on_progress: Channel<ChecksumState>,
) -> Result<ChecksumState, String> {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn_blocking(move || {
        // A flag the hashing loop polls; a watcher flips it when a newer request arrives.
        let cancel = Arc::new(AtomicBool::new(false));
        let superseded = || GENERATION.load(Ordering::SeqCst) != generation;
        let actual = sha256_file(&path, &cancel, &mut |bytes, total| {
            if superseded() {
                cancel.store(true, Ordering::Relaxed);
            }
            let _ = on_progress.send(ChecksumState::Hashing { bytes, total });
        })
        .map_err(|e| e.to_string())?;

        let pasted = expected.as_deref().and_then(normalize);
        let (expected, source) = match pasted {
            Some(h) => (h, "pasted".to_string()),
            None => match find_published(&path) {
                Some(found) => found,
                None => return Ok(ChecksumState::Unverified { actual }),
            },
        };
        Ok(if actual == expected {
            ChecksumState::Match { actual, source }
        } else {
            ChecksumState::Mismatch {
                actual,
                expected,
                source,
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
