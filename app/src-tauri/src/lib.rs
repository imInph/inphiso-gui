mod commands;
mod flash;

use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(flash::FlashState::default()))
        .invoke_handler(tauri::generate_handler![
            commands::list_drives,
            commands::flash,
            commands::cancel_flash,
            commands::eject,
        ])
        .run(tauri::generate_context!())
        .expect("error while running inphiso");
}
