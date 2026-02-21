// Folder Watcher - Überwacht einen lokalen Ordner und lädt neue Dateien zu DocFlow hoch
// Nutzt notify-Crate für Filesystem-Events (inotify/FSEvents/ReadDirectoryChanges)

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Konfiguration für den Folder-Sync
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FolderSyncConfig {
    pub enabled: bool,
    pub watch_path: String,
    pub post_upload_action: PostUploadAction,
}

/// Aktion nach erfolgreichem Upload
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum PostUploadAction {
    MoveToSubfolder,  // In "uploaded" Unterordner verschieben
    Delete,           // Löschen
    Keep,             // Nichts tun (für Tests)
}

/// Status des Folder-Watchers
#[derive(Clone, Debug, Serialize)]
pub struct FolderSyncStatus {
    pub running: bool,
    pub watch_path: Option<String>,
    pub files_uploaded: u32,
    pub files_pending: u32,
    pub errors: u32,
    pub last_upload: Option<String>,
    pub last_error: Option<String>,
}

/// Backend-Response nach Upload
#[derive(Debug, Deserialize)]
struct FolderUploadResponse {
    success: bool,
    job_id: i64,
    filename: String,
    #[allow(dead_code)]
    file_size_mb: f64,
    duplicate: bool,
    message: String,
}

/// Erlaubte Datei-Endungen
const ALLOWED_EXTENSIONS: &[&str] = &["pdf", "jpg", "jpeg", "png", "tiff", "tif"];

/// Max. Dateigröße in Bytes (50 MB)
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

/// Folder Watcher
pub struct FolderWatcher {
    pub config: RwLock<FolderSyncConfig>,
    api_key: String,
    docflow_url: String,
    status: Arc<RwLock<FolderSyncStatus>>,
    known_hashes: RwLock<HashSet<String>>,
}

impl FolderWatcher {
    pub fn new(config: FolderSyncConfig, api_key: String, docflow_url: String) -> Self {
        Self {
            config: RwLock::new(config),
            api_key,
            docflow_url,
            status: Arc::new(RwLock::new(FolderSyncStatus {
                running: false,
                watch_path: None,
                files_uploaded: 0,
                files_pending: 0,
                errors: 0,
                last_upload: None,
                last_error: None,
            })),
            known_hashes: RwLock::new(HashSet::new()),
        }
    }

    /// Prüft ob eine Datei eine erlaubte Endung hat
    fn is_allowed_extension(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ALLOWED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }

    /// Berechnet SHA256-Hash einer Datei
    async fn compute_file_hash(path: &Path) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let data = tokio::fs::read(path).await?;
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let hash = hasher.finalize();
        Ok(format!("{:x}", hash))
    }

    /// Wartet bis eine Datei stabil ist (nicht mehr geschrieben wird)
    async fn wait_for_file_stable(path: &Path) -> bool {
        let mut sizes = Vec::new();
        for _ in 0..3 {
            match tokio::fs::metadata(path).await {
                Ok(meta) => sizes.push(meta.len()),
                Err(_) => return false,
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        }
        sizes.len() == 3 && sizes[0] == sizes[1] && sizes[1] == sizes[2] && sizes[0] > 0
    }

    /// Lädt eine Datei zum DocFlow-Server hoch
    async fn upload_file(
        &self,
        path: &Path,
        file_hash: &str,
    ) -> Result<FolderUploadResponse, Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::new();
        let url = format!("{}/api/scanner/bridge/folder-upload", self.docflow_url);

        let data = tokio::fs::read(path).await?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let mime_type = match path.extension().and_then(|e| e.to_str()) {
            Some("pdf") => "application/pdf",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("png") => "image/png",
            Some("tiff") | Some("tif") => "image/tiff",
            _ => "application/octet-stream",
        };

        use reqwest::multipart::{Form, Part};
        let file_part = Part::bytes(data)
            .file_name(filename.clone())
            .mime_str(mime_type)?;

        let original_path = path.to_string_lossy().to_string();

        let form = Form::new()
            .part("file", file_part)
            .text("file_hash", file_hash.to_string())
            .text("original_path", original_path);

        // Retry-Logik: 3 Versuche mit exponentiellem Backoff
        let mut last_error = String::new();
        for attempt in 0..3u32 {
            if attempt > 0 {
                let delay = 2u64.pow(attempt);
                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
            }

            // Form muss für jeden Versuch neu gebaut werden
            let file_data = tokio::fs::read(path).await?;
            let retry_file_part = Part::bytes(file_data)
                .file_name(filename.clone())
                .mime_str(mime_type)?;
            let retry_form = Form::new()
                .part("file", retry_file_part)
                .text("file_hash", file_hash.to_string())
                .text("original_path", path.to_string_lossy().to_string());

            match client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .multipart(retry_form)
                .timeout(std::time::Duration::from_secs(60))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        let result: FolderUploadResponse = response.json().await?;
                        return Ok(result);
                    } else if response.status().as_u16() == 429 {
                        // Rate-Limit: Länger warten
                        last_error = "Rate-Limit erreicht".to_string();
                        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                        continue;
                    } else {
                        last_error = response.text().await.unwrap_or_default();
                        continue;
                    }
                }
                Err(e) => {
                    last_error = e.to_string();
                    continue;
                }
            }
        }

        Err(format!("Upload fehlgeschlagen nach 3 Versuchen: {}", last_error).into())
    }

    /// Verarbeitet eine einzelne Datei
    async fn process_file(&self, path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Extension prüfen
        if !Self::is_allowed_extension(path) {
            return Ok(()); // Ignorieren, kein Fehler
        }

        // Dateigröße prüfen
        let metadata = tokio::fs::metadata(path).await?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(format!(
                "Datei zu groß: {} MB (max {} MB)",
                metadata.len() / 1024 / 1024,
                MAX_FILE_SIZE / 1024 / 1024
            ).into());
        }

        // Warten bis Datei stabil ist
        if !Self::wait_for_file_stable(path).await {
            return Err("Datei nicht stabil (wird noch geschrieben?)".into());
        }

        // SHA256 berechnen
        let file_hash = Self::compute_file_hash(path).await?;

        // Lokal auf Duplikate prüfen
        {
            let hashes = self.known_hashes.read().await;
            if hashes.contains(&file_hash) {
                println!("⏭ Datei bereits hochgeladen (Hash bekannt): {}", path.display());
                // Trotzdem verschieben/löschen
                self.post_upload_action(path).await?;
                return Ok(());
            }
        }

        // Hochladen
        println!("📤 Lade hoch: {}", path.display());
        let result = self.upload_file(path, &file_hash).await?;

        // Hash merken
        {
            let mut hashes = self.known_hashes.write().await;
            hashes.insert(file_hash);
        }

        if result.duplicate {
            println!("⏭ Server: Duplikat (Job #{})", result.job_id);
        } else {
            println!("✓ Hochgeladen: {} → Job #{} ({})", result.filename, result.job_id, result.message);
        }

        // Status aktualisieren
        {
            let mut status = self.status.write().await;
            status.files_uploaded += 1;
            status.last_upload = Some(chrono::Utc::now().to_rfc3339());
        }

        // Post-Upload-Aktion
        self.post_upload_action(path).await?;

        Ok(())
    }

    /// Führt die konfigurierte Post-Upload-Aktion aus
    async fn post_upload_action(&self, path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let config = self.config.read().await;
        match config.post_upload_action {
            PostUploadAction::MoveToSubfolder => {
                let parent = path.parent().unwrap_or(Path::new("."));
                let uploaded_dir = parent.join("uploaded");
                tokio::fs::create_dir_all(&uploaded_dir).await?;
                let dest = uploaded_dir.join(path.file_name().unwrap_or_default());
                tokio::fs::rename(path, &dest).await?;
                println!("  → Verschoben nach: {}", dest.display());
            }
            PostUploadAction::Delete => {
                tokio::fs::remove_file(path).await?;
                println!("  → Gelöscht");
            }
            PostUploadAction::Keep => {
                // Nichts tun
            }
        }
        Ok(())
    }

    /// Meldet den Status an DocFlow
    async fn report_status_to_server(&self) {
        let client = reqwest::Client::new();
        let url = format!("{}/api/scanner/bridge/folder-sync-status", self.docflow_url);

        let status = self.status.read().await;
        let config = self.config.read().await;

        let body = serde_json::json!({
            "folder_sync_enabled": config.enabled,
            "watched_folder": config.watch_path,
            "files_uploaded": status.files_uploaded,
            "errors": status.errors,
            "last_sync_at": status.last_upload,
        });

        let _ = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
    }

    /// Startet den Folder-Watcher (Polling-basiert für maximale Kompatibilität)
    /// Nutzt Polling statt notify-Events, da SMB-Shares keine Events generieren
    pub async fn start_watching(self: Arc<Self>) {
        let config = self.config.read().await;
        let watch_path = PathBuf::from(&config.watch_path);
        drop(config);

        if !watch_path.exists() {
            eprintln!("❌ Ordner existiert nicht: {}", watch_path.display());
            let mut status = self.status.write().await;
            status.last_error = Some(format!("Ordner nicht gefunden: {}", watch_path.display()));
            return;
        }

        {
            let mut status = self.status.write().await;
            status.running = true;
            status.watch_path = Some(watch_path.to_string_lossy().to_string());
        }

        println!("📁 Folder-Sync gestartet: {}", watch_path.display());

        // Hauptschleife: Polling alle 5 Sekunden
        loop {
            // Stop-Flag prüfen
            {
                let status = self.status.read().await;
                if !status.running {
                    break;
                }
            }

            // Ordner scannen
            match tokio::fs::read_dir(&watch_path).await {
                Ok(mut entries) => {
                    let mut pending_count = 0u32;

                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let path = entry.path();

                        // Nur Dateien, keine Unterordner (uploaded/ ignorieren)
                        if !path.is_file() {
                            continue;
                        }

                        // uploaded/ Ordner überspringen
                        if path.parent()
                            .and_then(|p| p.file_name())
                            .and_then(|n| n.to_str())
                            == Some("uploaded")
                        {
                            continue;
                        }

                        if !Self::is_allowed_extension(&path) {
                            continue;
                        }

                        pending_count += 1;

                        // Datei verarbeiten
                        match self.process_file(&path).await {
                            Ok(()) => {}
                            Err(e) => {
                                eprintln!("❌ Fehler bei {}: {}", path.display(), e);
                                let mut status = self.status.write().await;
                                status.errors += 1;
                                status.last_error = Some(format!(
                                    "{}: {}", path.file_name().unwrap_or_default().to_string_lossy(), e
                                ));
                            }
                        }
                    }

                    {
                        let mut status = self.status.write().await;
                        status.files_pending = pending_count;
                    }
                }
                Err(e) => {
                    eprintln!("❌ Ordner nicht lesbar: {}", e);
                    let mut status = self.status.write().await;
                    status.last_error = Some(format!("Ordner nicht lesbar: {}", e));
                    status.errors += 1;
                }
            }

            // Status an Server melden (alle 30 Sekunden = 6 Zyklen)
            static CYCLE_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let cycle = CYCLE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if cycle % 6 == 0 {
                self.report_status_to_server().await;
            }

            // 5 Sekunden warten
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }

        println!("🛑 Folder-Sync gestoppt");

        // Letzten Status melden
        self.report_status_to_server().await;
    }

    /// Stoppt den Watcher
    pub async fn stop(&self) {
        let mut status = self.status.write().await;
        status.running = false;

        // Disabled-Status an Server melden
        let config = self.config.read().await;
        let client = reqwest::Client::new();
        let url = format!("{}/api/scanner/bridge/folder-sync-status", self.docflow_url);
        let body = serde_json::json!({
            "folder_sync_enabled": false,
            "watched_folder": config.watch_path,
            "files_uploaded": status.files_uploaded,
            "errors": status.errors,
        });
        drop(config);
        drop(status);

        let _ = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;
    }

    /// Gibt aktuellen Status zurück
    pub async fn get_status(&self) -> FolderSyncStatus {
        self.status.read().await.clone()
    }
}
