//! Tauri commands called from the React frontend (see `app/src/backend/tauri.ts`).
//! Errors cross the bridge as plain strings, which the UI shows as-is.

use inphiso_platform::Drive;

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
