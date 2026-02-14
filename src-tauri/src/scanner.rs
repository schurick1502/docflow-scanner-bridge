// Scanner-Modul - Scan-Operationen ausführen
// Platzhalter für zukünftige Implementierung

use serde::{Deserialize, Serialize};

/// Scan-Auftrag
#[derive(Debug, Deserialize)]
pub struct ScanJob {
    pub scanner_id: String,
    pub resolution: u32,
    pub color_mode: String,
    pub format: String,
    pub source: String, // flatbed, adf
    pub duplex: bool,
}

/// Scan-Ergebnis
#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub job_id: String,
    pub pages: Vec<ScannedPage>,
    pub total_pages: usize,
}

/// Gescannte Seite
#[derive(Debug, Serialize)]
pub struct ScannedPage {
    pub page_number: usize,
    pub format: String,
    pub size_bytes: usize,
    pub data_base64: String,
}

/// Führt Scan auf Netzwerk-Scanner via eSCL aus
pub async fn scan_escl(
    scanner_ip: &str,
    scanner_port: u16,
    job: &ScanJob,
) -> Result<ScanResult, Box<dyn std::error::Error + Send + Sync>> {
    scan_escl_with_tls(scanner_ip, scanner_port, false, job).await
}

/// Führt Scan auf Netzwerk-Scanner via eSCL aus (mit optionalem TLS)
pub async fn scan_escl_with_tls(
    scanner_ip: &str,
    scanner_port: u16,
    use_tls: bool,
    job: &ScanJob,
) -> Result<ScanResult, Box<dyn std::error::Error + Send + Sync>> {
    // HTTPS für TLS oder Port 443, selbstsignierte Zertifikate akzeptieren
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let scheme = if use_tls || scanner_port == 443 { "https" } else { "http" };

    // IPv6-Adressen brauchen Brackets in URLs
    let host = if scanner_ip.contains(':') {
        format!("[{}]", scanner_ip)
    } else {
        scanner_ip.to_string()
    };

    let base_url = format!("{}://{}:{}/eSCL", scheme, host, scanner_port);

    // 1. Scan-Job erstellen
    let scan_settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<scan:ScanSettings xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03"
                   xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">
    <pwg:Version>2.0</pwg:Version>
    <scan:Intent>Document</scan:Intent>
    <pwg:ScanRegions>
        <pwg:ScanRegion>
            <pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>
            <pwg:XOffset>0</pwg:XOffset>
            <pwg:YOffset>0</pwg:YOffset>
            <pwg:Width>2550</pwg:Width>
            <pwg:Height>3300</pwg:Height>
        </pwg:ScanRegion>
    </pwg:ScanRegions>
    <pwg:InputSource>{}</pwg:InputSource>
    <scan:ColorMode>{}</scan:ColorMode>
    <scan:XResolution>{}</scan:XResolution>
    <scan:YResolution>{}</scan:YResolution>
    <pwg:DocumentFormat>{}</pwg:DocumentFormat>
</scan:ScanSettings>"#,
        if job.source == "adf" { "Feeder" } else { "Platen" },
        job.color_mode,
        job.resolution,
        job.resolution,
        job.format
    );

    // Vor dem Scan: Scanner-Status prüfen und ggf. alte Jobs aufräumen
    println!("🔍 Prüfe Scanner-Status...");
    if let Ok(status_resp) = client.get(format!("{}/ScannerStatus", base_url)).send().await {
        if let Ok(status_xml) = status_resp.text().await {
            println!("📋 Scanner-Status: {}",
                if status_xml.contains("Idle") { "Idle" }
                else if status_xml.contains("Processing") { "Processing" }
                else if status_xml.contains("Testing") { "Testing" }
                else { "Unbekannt" }
            );

            // Bestehende Jobs aus ScannerStatus extrahieren und löschen
            // eSCL gibt JobInfo-Elemente mit JobUri zurück
            for line in status_xml.lines() {
                if line.contains("JobUri") || line.contains("jobUri") {
                    // JobUri extrahieren: <scan:JobUri>/eSCL/ScanJobs/42</scan:JobUri>
                    if let Some(start) = line.find("/eSCL/") {
                        let uri_part = &line[start..];
                        if let Some(end) = uri_part.find('<') {
                            let job_path = &uri_part[..end];
                            let delete_url = format!("{}://{}{}", scheme, host, job_path);
                            println!("🗑 Lösche hängenden Job: {}", job_path);
                            let _ = client.delete(&delete_url).send().await;
                        }
                    }
                }
            }
        }
    }

    // Scan-Job erstellen mit Retry bei 409 Conflict (Scanner busy)
    let mut job_url = String::new();
    let max_retries = 4;

    for attempt in 0..max_retries {
        if attempt > 0 {
            println!("⏳ Scanner busy (409), Versuch {}/{}...", attempt + 1, max_retries);
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

            // Bei 2. Retry: Aggressiv alle Jobs löschen die wir finden können
            if attempt >= 2 {
                println!("🔄 Versuche alle bestehenden Scan-Jobs zu löschen...");
                // Typische Job-IDs sind aufsteigend: versuche 1-20 zu löschen
                for job_num in 1..=20 {
                    let del_url = format!("{}/ScanJobs/{}", base_url, job_num);
                    let _ = client.delete(&del_url).send().await;
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }

        let response = client
            .post(format!("{}/ScanJobs", base_url))
            .header("Content-Type", "application/xml")
            .body(scan_settings.clone())
            .send()
            .await?;

        let status = response.status();

        if status.is_success() {
            job_url = response
                .headers()
                .get("Location")
                .and_then(|v| v.to_str().ok())
                .ok_or("Keine Job-URL erhalten")?
                .to_string();
            println!("✓ Scan-Job erstellt: {}", job_url);
            break;
        } else if status.as_u16() == 409 && attempt < max_retries - 1 {
            continue;
        } else {
            return Err(format!("Scan-Job erstellen fehlgeschlagen: {}", status).into());
        }
    }

    if job_url.is_empty() {
        return Err("Scanner dauerhaft busy (409 Conflict) — bitte Scanner neu starten oder Display prüfen".into());
    }

    // 2. Auf Scan-Ergebnis warten
    let mut pages = Vec::new();
    let mut page_number = 1;

    loop {
        // NextDocument abrufen
        let doc_url = format!("{}/NextDocument", job_url);
        let doc_response = client.get(&doc_url).send().await?;

        if doc_response.status().as_u16() == 404 {
            // Keine weiteren Seiten
            break;
        }

        if !doc_response.status().is_success() {
            // Scan noch nicht fertig, warten
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            continue;
        }

        let data = doc_response.bytes().await?;
        use base64::Engine;
        let data_base64 = base64::engine::general_purpose::STANDARD.encode(&data);

        pages.push(ScannedPage {
            page_number,
            format: job.format.clone(),
            size_bytes: data.len(),
            data_base64,
        });

        page_number += 1;
    }

    Ok(ScanResult {
        job_id: uuid::Uuid::new_v4().to_string(),
        total_pages: pages.len(),
        pages,
    })
}

// Platzhalter für native Scanner-Zugriffe
#[cfg(target_os = "windows")]
pub mod wia {
    //! Windows Image Acquisition
    pub async fn scan() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        todo!("WIA-Implementierung")
    }
}

#[cfg(target_os = "linux")]
pub mod sane {
    //! SANE Scanner Access
    pub async fn scan() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        todo!("SANE-Implementierung")
    }
}

#[cfg(target_os = "macos")]
pub mod image_capture {
    //! ImageCaptureCore
    pub async fn scan() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        todo!("ImageCaptureCore-Implementierung")
    }
}
