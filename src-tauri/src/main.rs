#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod backend;
use std::sync::Arc;
use tauri::Manager;
fn run() -> Result<(), String> {
    if rl_core::lifecycle::early_mode()? { return Ok(()); }
    let state = Arc::new(backend::Backend::new()?);
    let installer = state.is_installer();
    let builder = tauri::Builder::default().manage(state).plugin(tauri_plugin_dialog::init());
    let builder = if installer { builder } else { builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") { let _ = window.unminimize(); let _ = window.set_focus(); }
        })) };
    builder
        .setup(|app| {
            let state = app.state::<Arc<backend::Backend>>();
            let profile = state.webview_directory().map_err(std::io::Error::other)?;
            let config = app.config().app.windows.first().ok_or_else(|| std::io::Error::other("Missing main window configuration"))?;
            tauri::WebviewWindowBuilder::from_config(app, config)?.data_directory(profile).build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![backend::snapshot, backend::save_settings, backend::inspect_mo2, backend::choose_folder, backend::check_build, backend::install_build, backend::prepare_game, backend::launch_game, backend::verify_build, backend::cancel_job, backend::open_folder, backend::check_launcher_update, backend::apply_launcher_update, backend::default_install_dir, backend::set_installer_language, backend::install_app, backend::open_installed])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<Arc<backend::Backend>>();
                if state.busy.load(std::sync::atomic::Ordering::Relaxed) { api.prevent_close(); }
            }
        })
        .run(tauri::generate_context!()).map_err(|err| err.to_string())
}
fn main() {
    if let Err(err) = run() {
        eprintln!("RayLink: {err}");
        #[cfg(windows)] unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
            let message: Vec<u16> = format!("RayLink could not start.\n\n{err}").encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "RayLink".encode_utf16().chain(Some(0)).collect();
            MessageBoxW(std::ptr::null_mut(), message.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
        }
        std::process::exit(1);
    }
}
