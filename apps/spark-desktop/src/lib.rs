#![forbid(unsafe_code)]

use std::sync::Mutex;

use desktop_core::{
    bootstrap_desktop_runtime, load_desktop_settings, set_remote_access_enabled, settings_view,
    start_desktop_server, DesktopPaths, DesktopServer, DesktopServerSettings,
    DesktopServerSettingsView,
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

pub mod desktop_core;

struct DesktopAppState {
    paths: DesktopPaths,
    settings: Mutex<DesktopServerSettings>,
    server: Mutex<DesktopServer>,
    current_bind_host: String,
    server_url: String,
}

#[tauri::command]
fn desktop_server_settings(
    state: tauri::State<'_, DesktopAppState>,
) -> Result<DesktopServerSettingsView, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Desktop settings lock is poisoned.".to_string())?;
    Ok(settings_view(
        &settings,
        &state.current_bind_host,
        state.server_url.clone(),
    ))
}

#[tauri::command]
fn set_desktop_remote_access_enabled(
    state: tauri::State<'_, DesktopAppState>,
    enabled: bool,
    confirmed_warning: bool,
) -> Result<DesktopServerSettingsView, String> {
    let updated = set_remote_access_enabled(&state.paths, enabled, confirmed_warning)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "Desktop settings lock is poisoned.".to_string())?;
    *settings = updated;
    Ok(settings_view(
        &settings,
        &state.current_bind_host,
        state.server_url.clone(),
    ))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            desktop_server_settings,
            set_desktop_remote_access_enabled
        ])
        .setup(|app| {
            let paths = tauri_desktop_paths(app.handle()).map_err(boxed_error)?;
            let desktop_settings = load_desktop_settings(&paths).map_err(boxed_error)?;
            let bootstrap =
                bootstrap_desktop_runtime(&paths, &desktop_settings).map_err(boxed_error)?;
            let server = start_desktop_server(bootstrap.settings, &bootstrap.bind_host)
                .map_err(boxed_error)?;
            let server_url = server.url().to_string();
            let webview_url = server_url
                .parse()
                .map_err(|error| boxed_error(format!("Invalid desktop server URL: {error}")))?;
            app.manage(DesktopAppState {
                paths,
                settings: Mutex::new(desktop_settings),
                server: Mutex::new(server),
                current_bind_host: bootstrap.bind_host,
                server_url,
            });
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(webview_url))
                .title("Spark")
                .inner_size(1280.0, 840.0)
                .min_inner_size(960.0, 640.0)
                .build()
                .map_err(|error| boxed_error(format!("Unable to create Spark window: {error}")))?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let state = window.state::<DesktopAppState>();
                let server_lock = state.server.lock();
                if let Ok(mut server) = server_lock {
                    server.request_shutdown();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run Spark desktop app");
}

fn tauri_desktop_paths<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<DesktopPaths, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Unable to resolve app data directory: {error}"))?;
    let app_config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("Unable to resolve app config directory: {error}"))?;
    Ok(DesktopPaths::new(app_data_dir, app_config_dir))
}

fn boxed_error(message: String) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(std::io::ErrorKind::Other, message))
}
