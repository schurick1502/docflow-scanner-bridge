// Scan-Job-Poller - Holt Scan-Aufträge von DocFlow und führt sie aus
// Polling-Modell: Bridge fragt DocFlow regelmäßig nach neuen Jobs

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::discovery::DiscoveredScanner;
use crate::scanner::{scan_escl_with_tls, ScanJob};

/// Pending Scan-Job von DocFlow
#[derive(Debug, Deserialize, Clone)]
pub struct PendingScanJob {
    pub job_id: String,
    pub scanner_id: String,
    pub resolution: u32,
    pub color_mode: String,
    pub source: String,
    pub duplex: bool,
    pub format: String,
    pub created_at: String,
    pub expires_at: String,
}

/// Response von pending-scans Endpoint
#[derive(Debug, Deserialize)]
struct PendingScansResponse {
    jobs: Vec<PendingScanJob>,
}

/// Poller-Status
#[derive(Clone, Debug, Serialize)]
pub struct PollerStatus {
    pub running: bool,
    pub last_poll: Option<String>,
    pub jobs_processed: u32,
    pub last_error: Option<String>,
}

/// Scan-Job-Poller
pub struct ScanPoller {
    api_key: String,
    docflow_url: String,
    scanners: Arc<RwLock<Vec<DiscoveredScanner>>>,
    status: Arc<RwLock<PollerStatus>>,
}

impl ScanPoller {
    pub fn new(
        api_key: String,
        docflow_url: String,
        scanners: Arc<RwLock<Vec<DiscoveredScanner>>>,
    ) -> Self {
        Self {
            api_key,
            docflow_url,
            scanners,
            status: Arc::new(RwLock::new(PollerStatus {
                running: false,
                last_poll: None,
                jobs_processed: 0,
                last_error: None,
            })),
        }
    }

    /// Holt ausstehende Scan-Jobs von DocFlow
    pub async fn poll_pending_jobs(&self) -> Result<Vec<PendingScanJob>, Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::new();
        let url = format!("{}/api/scanner/bridge/pending-scans", self.docflow_url);

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Polling fehlgeschlagen: {}", error_text).into());
        }

        let result: PendingScansResponse = response.json().await?;
        Ok(result.jobs)
    }

    /// Führt einen Scan-Job aus
    pub async fn execute_scan_job(&self, job: &PendingScanJob) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Scanner finden
        let scanners = self.scanners.read().await;
        let scanner = scanners
            .iter()
            .find(|s| s.id == job.scanner_id)
            .ok_or_else(|| format!("Scanner '{}' nicht gefunden", job.scanner_id))?;

        println!("📄 Starte Scan auf {} ({})...", scanner.name, scanner.ip);

        // Scan durchführen
        let scan_job = ScanJob {
            scanner_id: job.scanner_id.clone(),
            resolution: job.resolution,
            color_mode: job.color_mode.clone(),
            format: if job.format == "pdf" { "application/pdf".to_string() } else { "image/jpeg".to_string() },
            source: job.source.clone(),
            duplex: job.duplex,
        };

        let result = scan_escl_with_tls(&scanner.ip, scanner.port, scanner.use_tls, &scanner.rs_path, &scan_job).await?;

        if result.pages.is_empty() {
            return Err("Keine Seiten gescannt".into());
        }

        // Wenn PDF: Alle Seiten zusammenfügen (oder erste Seite nehmen wenn schon PDF)
        // Für den Moment: Erste Seite nehmen
        let first_page = &result.pages[0];
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD
            .decode(&first_page.data_base64)?;

        println!("✓ Scan abgeschlossen: {} Seiten, {} Bytes", result.total_pages, data.len());

        Ok(data)
    }

    /// Lädt Scan-Ergebnis zu DocFlow hoch
    pub async fn upload_scan_result(
        &self,
        job_id: &str,
        data: Vec<u8>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::new();
        let url = format!("{}/api/scanner/bridge/scan-upload/{}", self.docflow_url, job_id);

        // Multipart-Form erstellen
        use reqwest::multipart::{Form, Part};

        let file_part = Part::bytes(data)
            .file_name("scan.pdf")
            .mime_str("application/pdf")?;

        let form = Form::new()
            .part("file", file_part)
            .text("success", "true");

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Upload fehlgeschlagen: {}", error_text).into());
        }

        println!("✓ Scan hochgeladen: Job {}", job_id);
        Ok(())
    }

    /// Meldet einen Fehler an DocFlow
    pub async fn report_error(
        &self,
        job_id: &str,
        error_message: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::new();
        let url = format!("{}/api/scanner/bridge/scan-upload/{}", self.docflow_url, job_id);

        use reqwest::multipart::{Form, Part};

        // Leere Datei mit Fehler
        let empty_part = Part::bytes(vec![])
            .file_name("error.txt")
            .mime_str("text/plain")?;

        let form = Form::new()
            .part("file", empty_part)
            .text("success", "false")
            .text("error_message", error_message.to_string());

        let _ = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        Ok(())
    }

    /// Startet den Polling-Loop
    pub async fn start_polling(self: Arc<Self>) {
        {
            let mut status = self.status.write().await;
            status.running = true;
        }

        println!("🔄 Scan-Job-Poller gestartet");

        loop {
            // Status prüfen
            {
                let status = self.status.read().await;
                if !status.running {
                    break;
                }
            }

            // Polling durchführen
            match self.poll_pending_jobs().await {
                Ok(jobs) => {
                    {
                        let mut status = self.status.write().await;
                        status.last_poll = Some(chrono::Utc::now().to_rfc3339());
                        status.last_error = None;
                    }

                    for job in jobs {
                        println!("📥 Neuer Scan-Job: {} (Scanner: {})", job.job_id, job.scanner_id);

                        // Scan ausführen
                        match self.execute_scan_job(&job).await {
                            Ok(data) => {
                                // Upload
                                if let Err(e) = self.upload_scan_result(&job.job_id, data).await {
                                    eprintln!("❌ Upload fehlgeschlagen: {}", e);
                                    let _ = self.report_error(&job.job_id, &e.to_string()).await;
                                } else {
                                    let mut status = self.status.write().await;
                                    status.jobs_processed += 1;
                                }
                            }
                            Err(e) => {
                                eprintln!("❌ Scan fehlgeschlagen: {}", e);
                                let _ = self.report_error(&job.job_id, &e.to_string()).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    let mut status = self.status.write().await;
                    status.last_error = Some(e.to_string());
                    // Bei Fehler nicht sofort aufgeben, nur loggen
                    if !e.to_string().contains("401") {
                        eprintln!("⚠ Polling-Fehler: {}", e);
                    }
                }
            }

            // Warten vor nächstem Poll (2 Sekunden)
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }

        println!("🛑 Scan-Job-Poller gestoppt");
    }

    /// Stoppt den Poller
    pub async fn stop(&self) {
        let mut status = self.status.write().await;
        status.running = false;
    }

    /// Gibt aktuellen Status zurück
    pub async fn get_status(&self) -> PollerStatus {
        self.status.read().await.clone()
    }
}
