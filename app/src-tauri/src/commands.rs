//! Tauri commands called from the React frontend (see `app/src/backend/tauri.ts`).
//! Errors cross the bridge as plain strings, which the UI shows as-is.

use std::sync::Arc;

use inphiso_core::ipc::HelperMsg;
use inphiso_platform::Drive;
use tauri::ipc::Channel;
use tauri::State;

use crate::flash::{self, FlashJob, FlashState};

type CmdResult<T> = Result<T, String>;

/// Runs blocking work (shelling out to diskutil / lsblk, IOCTLs) off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn list_drives(show_all: bool) -> CmdResult<Vec<Drive>> {
    blocking(move || Ok(inphiso_platform::list_drives(show_all)?)).await
}

/// Resolves when the flash is over; progress and the outcome arrive on `on_event`.
#[tauri::command]
pub async fn flash(
    state: State<'_, Arc<FlashState>>,
    job: FlashJob,
    on_event: Channel<HelperMsg>,
) -> CmdResult<()> {
    let state = Arc::clone(&state);
    tauri::async_runtime::spawn_blocking(move || flash::run(&state, job, &on_event))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_flash(state: State<'_, Arc<FlashState>>) {
    state.cancel();
}

#[tauri::command]
pub async fn eject(drive_id: String) -> CmdResult<()> {
    blocking(move || Ok(inphiso_platform::eject(&drive_id)?)).await
}
