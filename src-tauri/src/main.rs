// DocFlow Scanner Bridge - Hauptanwendung
// Verbindet lokale Scanner mit DocFlow (Docker/Cloud)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod discovery;
mod folder_watcher;
mod pairing;
mod scanner;
mod scan_poller;

use std::sync::Arc;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};
use tauri_plugin_autostart::MacosLauncher;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use serde_json;
use reqwest;

use folder_watcher::{FolderSyncConfig, FolderSyncStatus, FolderWatcher, PostUploadAction};
use scan_poller::ScanPoller;

/// Bridge-Status für das Frontend
#[derive(Clone, Serialize, Deserialize)]
pub struct BridgeStatus {
    connected: bool,
    docflow_url: Option<String>,
    scanner_count: usize,
    last_discovery: Option<String>,
    version: String,
    poller_active: bool,
    jobs_processed: u32,
    folder_sync_active: bool,
    folder_sync_path: Option<String>,
}

/// Globaler App-State
pub struct AppState {
    bridge_status: RwLock<BridgeStatus>,
    api_key: RwLock<Option<String>>,
    scanners: Arc<RwLock<Vec<discovery::DiscoveredScanner>>>,
    poller: RwLock<Option<Arc<ScanPoller>>>,
    folder_watcher: RwLock<Option<Arc<FolderWatcher>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            bridge_status: RwLock::new(BridgeStatus {
                connected: false,
                docflow_url: None,
                scanner_count: 0,
                last_discovery: None,
                version: env!("CARGO_PKG_VERSION").to_string(),
                poller_active: false,
                jobs_processed: 0,
                folder_sync_active: false,
                folder_sync_path: None,
            }),
            api_key: RwLock::new(None),
            scanners: Arc::new(RwLock::new(Vec::new())),
            poller: RwLock::new(None),
            folder_watcher: RwLock::new(None),
        }
    }
}

/// Tauri-Befehl: Status abrufen
#[tauri::command]
async fn get_status(state: tauri::State<'_, Arc<AppState>>) -> Result<BridgeStatus, String> {
    let status = state.bridge_status.read().await;
    Ok(status.clone())
}

/// Tauri-Befehl: Scanner suchen und an DocFlow senden
#[tauri::command]
async fn discover_scanners(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<discovery::DiscoveredScanner>, String> {
    let scanners = discovery::discover_all().await.map_err(|e| e.to_string())?;

    // Scanner im State speichern (für Poller)
    {
        let mut stored_scanners = state.scanners.write().await;
        *stored_scanners = scanners.clone();
    }

    // Status aktualisieren
    {
        let mut status = state.bridge_status.write().await;
        status.scanner_count = scanners.len();
        status.last_discovery = Some(chrono::Utc::now().to_rfc3339());
    }

    // Scanner an DocFlow senden (falls verbunden)
    let api_key = state.api_key.read().await.clone();
    let docflow_url = state.bridge_status.read().await.docflow_url.clone();

    if let (Some(key), Some(url)) = (api_key, docflow_url) {
        if let Err(e) = send_scanners_to_docflow(&url, &key, &scanners).await {
            eprintln!("Warnung: Konnte Scanner nicht an DocFlow senden: {}", e);
        }
    }

    Ok(scanners)
}

/// Sendet die gefundenen Scanner an DocFlow
async fn send_scanners_to_docflow(
    docflow_url: &str,
    api_key: &str,
    scanners: &[discovery::DiscoveredScanner]
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/scanner/bridge/scanners", docflow_url.trim_end_matches('/'));

    // Scanner-Daten für API aufbereiten
    let scanner_data: Vec<serde_json::Value> = scanners.iter().map(|s| {
        serde_json::json!({
            "id": s.id,
            "name": s.name,
            "manufacturer": s.manufacturer,
            "model": s.model,
            "ip": s.ip,
            "port": s.port,
            "protocols": s.protocols,
            "discovery_method": s.discovery_method,
            "capabilities": {
                "duplex": s.capabilities.duplex,
                "adf": s.capabilities.adf,
                "flatbed": s.capabilities.flatbed,
                "max_resolution": s.capabilities.max_resolution,
                "color_modes": s.capabilities.color_modes,
                "formats": s.capabilities.formats
            }
        })
    }).collect();

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&serde_json::json!({ "scanners": scanner_data }))
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("DocFlow-Fehler: {}", error_text).into());
    }

    println!("✓ {} Scanner an DocFlow gesendet", scanners.len());
    Ok(())
}

/// Tauri-Befehl: Mit DocFlow verbinden (Pairing)
/// docflow_url: Optional - nur für manuelle Codes benötigt (z.B. "http://localhost:4000")
#[tauri::command]
async fn pair_with_docflow(
    state: tauri::State<'_, Arc<AppState>>,
    pairing_code: String,
    docflow_url: Option<String>
) -> Result<bool, String> {
    // Pairing-Code parsen und mit DocFlow verbinden
    let result = pairing::pair(&pairing_code, docflow_url.as_deref()).await.map_err(|e| e.to_string())?;

    // API-Key und URL für Poller speichern
    let api_key_value = result.api_key.clone();
    let docflow_url_value = result.docflow_url.clone();

    // Status aktualisieren
    {
        let mut status = state.bridge_status.write().await;
        status.connected = true;
        status.docflow_url = Some(docflow_url_value.clone());
    }

    // API-Key sicher speichern
    {
        let mut api_key = state.api_key.write().await;
        *api_key = Some(api_key_value.clone());
    }

    // Scan-Poller starten
    let poller = Arc::new(ScanPoller::new(
        api_key_value,
        docflow_url_value,
        state.scanners.clone(),
    ));

    {
        let mut poller_lock = state.poller.write().await;
        *poller_lock = Some(poller.clone());
    }

    // Poller in separatem Task starten
    let poller_clone = poller.clone();
    tokio::spawn(async move {
        poller_clone.start_polling().await;
    });

    // Poller-Status im Bridge-Status aktualisieren
    {
        let mut status = state.bridge_status.write().await;
        status.poller_active = true;
    }

    println!("✓ Scan-Poller gestartet");

    Ok(true)
}

/// Tauri-Befehl: Verbindung trennen
#[tauri::command]
async fn disconnect(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    // Poller stoppen
    {
        let poller_lock = state.poller.read().await;
        if let Some(poller) = poller_lock.as_ref() {
            poller.stop().await;
        }
    }

    {
        let mut poller_lock = state.poller.write().await;
        *poller_lock = None;
    }

    // Folder-Watcher stoppen
    {
        let watcher_lock = state.folder_watcher.read().await;
        if let Some(watcher) = watcher_lock.as_ref() {
            watcher.stop().await;
        }
    }

    {
        let mut watcher_lock = state.folder_watcher.write().await;
        *watcher_lock = None;
    }

    let mut status = state.bridge_status.write().await;
    status.connected = false;
    status.docflow_url = None;
    status.poller_active = false;
    status.folder_sync_active = false;
    status.folder_sync_path = None;

    drop(status);

    let mut api_key = state.api_key.write().await;
    *api_key = None;

    // API-Key aus Keyring löschen
    if let Ok(entry) = keyring::Entry::new("docflow-scanner-bridge", "api_key") {
        if let Err(e) = entry.delete_password() {
            eprintln!("Warnung: Konnte API-Key nicht löschen: {}", e);
        }
    }

    println!("✓ Verbindung getrennt, Poller & Folder-Sync gestoppt");

    Ok(())
}

/// Tauri-Befehl: Ordner-Sync konfigurieren und starten
#[tauri::command]
async fn configure_folder_sync(
    state: tauri::State<'_, Arc<AppState>>,
    watch_path: String,
    post_action: String,
) -> Result<bool, String> {
    // Prüfe ob verbunden
    let api_key = state.api_key.read().await.clone();
    let docflow_url = state.bridge_status.read().await.docflow_url.clone();

    let (key, url) = match (api_key, docflow_url) {
        (Some(k), Some(u)) => (k, u),
        _ => return Err("Nicht mit DocFlow verbunden".to_string()),
    };

    // Prüfe ob Ordner existiert
    if !std::path::Path::new(&watch_path).exists() {
        return Err(format!("Ordner existiert nicht: {}", watch_path));
    }

    // Bestehenden Watcher stoppen
    {
        let watcher_lock = state.folder_watcher.read().await;
        if let Some(watcher) = watcher_lock.as_ref() {
            watcher.stop().await;
        }
    }

    let action = match post_action.as_str() {
        "delete" => PostUploadAction::Delete,
        "keep" => PostUploadAction::Keep,
        _ => PostUploadAction::MoveToSubfolder,
    };

    let config = FolderSyncConfig {
        enabled: true,
        watch_path: watch_path.clone(),
        post_upload_action: action,
    };

    // Config im Keyring speichern
    if let Ok(entry) = keyring::Entry::new("docflow-scanner-bridge", "folder_sync_config") {
        if let Ok(json) = serde_json::to_string(&config) {
            let _ = entry.set_password(&json);
        }
    }

    let watcher = Arc::new(FolderWatcher::new(config, key, url));

    {
        let mut watcher_lock = state.folder_watcher.write().await;
        *watcher_lock = Some(watcher.clone());
    }

    // Watcher in separatem Task starten
    let watcher_clone = watcher.clone();
    tokio::spawn(async move {
        watcher_clone.start_watching().await;
    });

    // Bridge-Status aktualisieren
    {
        let mut status = state.bridge_status.write().await;
        status.folder_sync_active = true;
        status.folder_sync_path = Some(watch_path);
    }

    println!("✓ Folder-Sync gestartet");
    Ok(true)
}

/// Tauri-Befehl: Ordner-Sync stoppen
#[tauri::command]
async fn stop_folder_sync(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    {
        let watcher_lock = state.folder_watcher.read().await;
        if let Some(watcher) = watcher_lock.as_ref() {
            watcher.stop().await;
        }
    }

    {
        let mut watcher_lock = state.folder_watcher.write().await;
        *watcher_lock = None;
    }

    // Config im Keyring deaktivieren
    if let Ok(entry) = keyring::Entry::new("docflow-scanner-bridge", "folder_sync_config") {
        if let Ok(json_str) = entry.get_password() {
            if let Ok(mut config) = serde_json::from_str::<FolderSyncConfig>(&json_str) {
                config.enabled = false;
                if let Ok(json) = serde_json::to_string(&config) {
                    let _ = entry.set_password(&json);
                }
            }
        }
    }

    {
        let mut status = state.bridge_status.write().await;
        status.folder_sync_active = false;
        status.folder_sync_path = None;
    }

    println!("✓ Folder-Sync gestoppt");
    Ok(())
}

/// Tauri-Befehl: Folder-Sync-Status abfragen
#[tauri::command]
async fn get_folder_sync_status(state: tauri::State<'_, Arc<AppState>>) -> Result<FolderSyncStatus, String> {
    let watcher_lock = state.folder_watcher.read().await;
    if let Some(watcher) = watcher_lock.as_ref() {
        Ok(watcher.get_status().await)
    } else {
        Ok(FolderSyncStatus {
            running: false,
            watch_path: None,
            files_uploaded: 0,
            files_pending: 0,
            errors: 0,
            last_upload: None,
            last_error: None,
        })
    }
}

/// Tauri-Befehl: Nativen Ordner-Dialog öffnen
#[tauri::command]
async fn pick_folder() -> Result<Option<String>, String> {
    let folder = rfd::AsyncFileDialog::new()
        .set_title("Scan-Ordner auswählen")
        .pick_folder()
        .await;

    Ok(folder.map(|f| f.path().to_string_lossy().to_string()))
}

/// Prüft auf Updates und zeigt ggf. einen Dialog
async fn check_for_updates(app: tauri::AppHandle) {
    use tauri_plugin_updater::UpdaterExt;

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Updater konnte nicht initialisiert werden: {}", e);
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            println!("Update verfügbar: v{}", update.version);
            if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
                eprintln!("Update-Installation fehlgeschlagen: {}", e);
            }
        }
        Ok(None) => {
            println!("Kein Update verfügbar - aktuelle Version ist aktuell");
        }
        Err(e) => {
            eprintln!("Update-Prüfung fehlgeschlagen: {}", e);
        }
    }
}

fn main() {
    let state = Arc::new(AppState::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .setup(|app| {
            // System Tray einrichten
            let tray_menu = tauri::menu::MenuBuilder::new(app)
                .text("status", "📡 Nicht verbunden")
                .separator()
                .text("discover", "🔍 Scanner suchen")
                .text("settings", "⚙️ Einstellungen")
                .separator()
                .text("update", "🔄 Nach Updates suchen")
                .separator()
                .text("quit", "Beenden")
                .build()?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("DocFlow Scanner Bridge")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "quit" => {
                            std::process::exit(0);
                        }
                        "settings" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "discover" => {
                            // Discovery in separatem Task starten
                            let app_handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Some(window) = app_handle.get_webview_window("main") {
                                    let _ = window.emit("discovery-started", ());
                                }
                            });
                        }
                        "update" => {
                            let app_handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                check_for_updates(app_handle).await;
                            });
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Fenster beim Schließen minimieren statt beenden
            let main_window = app.get_webview_window("main").unwrap();
            main_window.on_window_event(|event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    // Fenster verstecken statt schließen
                    api.prevent_close();
                }
            });

            // Auto-Update beim Start (nur in Release-Builds)
            #[cfg(not(debug_assertions))]
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Kurz warten, damit die App vollständig geladen ist
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    check_for_updates(app_handle).await;
                });
            }

            // Beim Start: Gespeicherten API-Key und DocFlow-URL laden
            let state = app.state::<Arc<AppState>>();
            let state_clone = state.inner().clone();
            tauri::async_runtime::spawn(async move {
                let api_key_result = keyring::Entry::new("docflow-scanner-bridge", "api_key")
                    .ok()
                    .and_then(|e| e.get_password().ok());
                let docflow_url_result = keyring::Entry::new("docflow-scanner-bridge", "docflow_url")
                    .ok()
                    .and_then(|e| e.get_password().ok());

                if let (Some(key), Some(url)) = (api_key_result, docflow_url_result) {
                    // API-Key und URL speichern
                    {
                        let mut api_key = state_clone.api_key.write().await;
                        *api_key = Some(key.clone());
                    }

                    {
                        let mut status = state_clone.bridge_status.write().await;
                        status.connected = true;
                        status.docflow_url = Some(url.clone());
                    }

                    // Scan-Poller starten
                    let poller = Arc::new(ScanPoller::new(
                        key,
                        url,
                        state_clone.scanners.clone(),
                    ));

                    {
                        let mut poller_lock = state_clone.poller.write().await;
                        *poller_lock = Some(poller.clone());
                    }

                    // Poller in separatem Task starten
                    let poller_clone = poller.clone();
                    tokio::spawn(async move {
                        poller_clone.start_polling().await;
                    });

                    {
                        let mut status = state_clone.bridge_status.write().await;
                        status.poller_active = true;
                    }

                    println!("✓ Verbindung wiederhergestellt, Poller gestartet");

                    // Folder-Sync Config laden und ggf. starten
                    let folder_config_result = keyring::Entry::new("docflow-scanner-bridge", "folder_sync_config")
                        .ok()
                        .and_then(|e| e.get_password().ok())
                        .and_then(|json| serde_json::from_str::<FolderSyncConfig>(&json).ok());

                    if let Some(config) = folder_config_result {
                        if config.enabled && std::path::Path::new(&config.watch_path).exists() {
                            let watcher = Arc::new(FolderWatcher::new(
                                config.clone(),
                                key.clone(),
                                url.clone(),
                            ));

                            {
                                let mut watcher_lock = state_clone.folder_watcher.write().await;
                                *watcher_lock = Some(watcher.clone());
                            }

                            let watcher_clone = watcher.clone();
                            tokio::spawn(async move {
                                watcher_clone.start_watching().await;
                            });

                            {
                                let mut status = state_clone.bridge_status.write().await;
                                status.folder_sync_active = true;
                                status.folder_sync_path = Some(config.watch_path);
                            }

                            println!("✓ Folder-Sync wiederhergestellt");
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            discover_scanners,
            pair_with_docflow,
            disconnect,
            configure_folder_sync,
            stop_folder_sync,
            get_folder_sync_status,
            pick_folder,
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Starten der Anwendung");
}
