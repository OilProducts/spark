use std::fs;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use serde::Serialize;
use spark_common::settings::SparkSettings;
use spark_server::{RuntimeInitializationOptions, SeedStarterFlowsResult};
use tokio::sync::oneshot;

pub const LOCAL_BIND_HOST: &str = "127.0.0.1";
pub const REMOTE_BIND_HOST: &str = "0.0.0.0";
pub const REMOTE_ACCESS_WARNING: &str =
    "Remote access lets other devices on this network reach the Spark desktop server.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopPaths {
    pub app_data_dir: PathBuf,
    pub app_config_dir: PathBuf,
}

impl DesktopPaths {
    pub fn new(app_data_dir: impl Into<PathBuf>, app_config_dir: impl Into<PathBuf>) -> Self {
        Self {
            app_data_dir: app_data_dir.into(),
            app_config_dir: app_config_dir.into(),
        }
    }
}

pub use spark_common::settings::DesktopSettings as DesktopServerSettings;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DesktopServerSettingsView {
    pub remote_access_enabled: bool,
    pub bind_host: String,
    pub server_url: String,
    pub requires_restart: bool,
    pub revision: String,
    pub remote_access_warning: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopBootstrap {
    pub settings: SparkSettings,
    pub seeded_flows: SeedStarterFlowsResult,
    pub bind_host: String,
}

pub struct DesktopServer {
    notify_settings_changed: Box<dyn Fn(&str) + Send + Sync>,
    url: String,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl DesktopServer {
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn save_remote_access(
        &self,
        paths: &DesktopPaths,
        enabled: bool,
        confirmed_warning: bool,
        expected_revision: &str,
    ) -> Result<(), String> {
        let document =
            save_remote_access_document(paths, enabled, confirmed_warning, expected_revision)?;
        (self.notify_settings_changed)(&document.revision);
        Ok(())
    }

    pub fn request_shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = self.thread.take();
    }

    pub fn shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for DesktopServer {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}

pub fn default_spark_data_dir(paths: &DesktopPaths) -> PathBuf {
    paths.app_data_dir.join("spark")
}

pub fn desktop_config_file(paths: &DesktopPaths) -> PathBuf {
    paths.app_config_dir.join("spark-desktop.json")
}

pub fn core_config_file(paths: &DesktopPaths) -> PathBuf {
    default_spark_data_dir(paths).join("config/spark.toml")
}

pub fn load_desktop_settings(paths: &DesktopPaths) -> Result<DesktopServerSettings, String> {
    let path = core_config_file(paths);
    spark_storage::settings::migrate_desktop_settings(&path, &desktop_config_file(paths))
        .and_then(|document| document.section(&path, "desktop"))
        .map(|settings| settings.unwrap_or_default())
        .map_err(|error| error.to_string())
}

pub fn desktop_settings_revision(paths: &DesktopPaths) -> Result<String, String> {
    spark_storage::settings::read_settings_document(&core_config_file(paths))
        .map(|document| document.revision)
        .map_err(|error| error.to_string())
}

pub fn read_desktop_settings_view(
    paths: &DesktopPaths,
    current_bind_host: &str,
    server_url: &str,
) -> Result<DesktopServerSettingsView, String> {
    let path = core_config_file(paths);
    let document =
        spark_storage::settings::migrate_desktop_settings(&path, &desktop_config_file(paths))
            .map_err(|error| error.to_string())?;
    let settings = document
        .section(&path, "desktop")
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    Ok(settings_view(
        &settings,
        current_bind_host,
        server_url,
        document.revision,
    ))
}

pub fn set_remote_access_enabled(
    paths: &DesktopPaths,
    enabled: bool,
    confirmed_warning: bool,
    expected_revision: &str,
) -> Result<DesktopServerSettings, String> {
    save_remote_access_document(paths, enabled, confirmed_warning, expected_revision)?;
    Ok(DesktopServerSettings {
        remote_access_enabled: enabled,
    })
}

fn save_remote_access_document(
    paths: &DesktopPaths,
    enabled: bool,
    confirmed_warning: bool,
    expected_revision: &str,
) -> Result<spark_storage::settings::SettingsDocument, String> {
    if enabled && !confirmed_warning {
        return Err("Remote access requires explicit warning confirmation.".to_string());
    }
    let settings = DesktopServerSettings {
        remote_access_enabled: enabled,
    };
    spark_storage::settings::update_desktop_settings(
        &core_config_file(paths),
        expected_revision,
        &settings,
    )
    .map_err(|error| error.to_string())
}

pub fn server_host_for_settings(settings: &DesktopServerSettings) -> &'static str {
    if settings.remote_access_enabled {
        REMOTE_BIND_HOST
    } else {
        LOCAL_BIND_HOST
    }
}

pub fn frontend_url_for_addr(addr: SocketAddr) -> String {
    let host = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => LOCAL_BIND_HOST.to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => LOCAL_BIND_HOST.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
        ip => ip.to_string(),
    };
    format!("http://{host}:{}/", addr.port())
}

pub fn settings_view(
    settings: &DesktopServerSettings,
    current_bind_host: &str,
    server_url: impl Into<String>,
    revision: String,
) -> DesktopServerSettingsView {
    let bind_host = server_host_for_settings(settings).to_string();
    DesktopServerSettingsView {
        requires_restart: bind_host != current_bind_host,
        remote_access_enabled: settings.remote_access_enabled,
        bind_host,
        server_url: server_url.into(),
        remote_access_warning: REMOTE_ACCESS_WARNING,
        revision,
    }
}

pub fn bootstrap_desktop_runtime(
    paths: &DesktopPaths,
    settings: &DesktopServerSettings,
) -> Result<DesktopBootstrap, String> {
    let data_dir = default_spark_data_dir(paths);
    fs::create_dir_all(&paths.app_data_dir).map_err(|error| {
        format!(
            "Unable to create desktop app data directory {}: {error}",
            paths.app_data_dir.display()
        )
    })?;
    let (spark_settings, seeded_flows) = spark_server::initialize_runtime_with_options(
        &RuntimeInitializationOptions {
            data_dir: Some(data_dir),
            flows_dir: None,
            ui_dir: None,
            force: false,
        },
        &spark_common::paths::ProcessEnvironment,
    )?;
    let bind_host = server_host_for_settings(settings).to_string();
    Ok(DesktopBootstrap {
        settings: spark_settings,
        seeded_flows,
        bind_host,
    })
}

pub fn start_desktop_server(
    mut settings: SparkSettings,
    bind_host: &str,
) -> Result<DesktopServer, String> {
    let listener = TcpListener::bind((bind_host, 0))
        .map_err(|error| format!("Unable to bind desktop server on {bind_host}:0: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Unable to configure desktop server listener: {error}"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("Unable to read desktop server address: {error}"))?;
    let url = frontend_url_for_addr(local_addr);
    // Desktop owns its listener and client target; remote access still requires
    // the native confirmation boundary. Report that effective choice separately.
    settings.startup_sources.insert(
        "connections.server_host".into(),
        "Desktop remote-access selection".into(),
    );
    settings.startup_sources.insert(
        "connections.server_port".into(),
        "Desktop automatic port".into(),
    );
    settings.connections.server_host = Some(bind_host.to_owned());
    settings.connections.server_port = Some(local_addr.port());
    settings.connections.client_api_base_url = Some(url.clone());
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (startup_tx, startup_rx) =
        mpsc::channel::<Result<Box<dyn Fn(&str) + Send + Sync>, String>>();
    let thread = thread::Builder::new()
        .name("spark-desktop-http".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = startup_tx.send(Err(format!(
                        "Unable to start desktop server runtime: {error}"
                    )));
                    return;
                }
            };
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = startup_tx.send(Err(format!(
                            "Unable to adopt desktop server listener: {error}"
                        )));
                        return;
                    }
                };
                let client = spark_server::rust_llm_client_from_settings(&settings);
                let (app, notify) =
                    spark_http::build_app_with_rust_llm_client_and_settings_notifications(
                        settings, client,
                    );
                let _ = startup_tx.send(Ok(Box::new(notify)));
                let server = axum::serve(listener, app).with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                });
                let _ = server.await;
            });
        })
        .map_err(|error| format!("Unable to spawn desktop server thread: {error}"))?;
    let notify_settings_changed = startup_rx
        .recv()
        .map_err(|error| format!("Desktop server startup channel closed: {error}"))??;
    Ok(DesktopServer {
        notify_settings_changed,
        url,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}

pub fn is_app_owned_data_dir(paths: &DesktopPaths, data_dir: &Path) -> bool {
    data_dir.starts_with(&paths.app_data_dir)
}

/// App-owned bootstrap identity survives ephemeral HTTP ports and WebView origins.
pub fn desktop_client_identity(paths: &DesktopPaths) -> Result<String, String> {
    spark_storage::settings::load_or_create_client_identity(&paths.app_config_dir.join("client-id"))
        .map_err(|error| error.to_string())
}
