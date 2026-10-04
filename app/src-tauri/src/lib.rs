mod commands;
mod download;
mod flash;
mod image;

use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // reqwest is built without a bundled crypto provider; use ring.
    let _ = rustls::crypto::ring::default_provider().install_default();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(flash::FlashState::default()))
        .manage(Arc::new(download::DownloadState::default()))
        .invoke_handler(tauri::generate_handler![
            commands::list_drives,
            commands::flash,
            commands::cancel_flash,
            commands::eject,
            image::inspect_image,
            image::checksum,
            download::download,
            download::cancel_download,
        ])
        .run(tauri::generate_context!())
        .expect("error while running inphiso");
}
