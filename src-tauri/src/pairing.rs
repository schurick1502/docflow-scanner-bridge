// Pairing-Modul - Verbindung mit DocFlow herstellen
// Unterstützt: QR-Code, manueller Token

use serde::{Deserialize, Serialize};

/// Pairing-Code Struktur (aus QR-Code oder manuelle Eingabe)
#[derive(Debug, Deserialize)]
pub struct PairingCode {
    pub docflow_url: String,
    #[serde(default)]
    pub tenant_id: Option<i64>,  // Integer wie in der DB, kann null sein bei Single-Tenant
    pub pairing_token: String,
    #[serde(default)]
    pub bridge_name: Option<String>,
}

/// Pairing-Ergebnis von DocFlow
#[derive(Debug, Deserialize)]
pub struct PairingResult {
    pub bridge_id: String,
    pub api_key: String,
    pub refresh_token: String,
    pub docflow_url: String,
    pub tenant_name: String,
}

/// Registrierungsanfrage an DocFlow
#[derive(Debug, Serialize)]
struct RegisterRequest {
    pairing_token: String,
    bridge_name: String,
    bridge_version: String,
    os: String,
    hostname: String,
}

/// Führt Pairing mit DocFlow durch
/// docflow_url: Optional - nur für manuelle Codes benötigt (z.B. "http://localhost:4000")
pub async fn pair(pairing_code: &str, docflow_url: Option<&str>) -> Result<PairingResult, Box<dyn std::error::Error + Send + Sync>> {
    // Pairing-Code parsen (JSON oder einfacher Token)
    // Für manuelle Codes: Benutzer-URL hat Priorität (Server-URL könnte Port fehlen)
    let (code, effective_url): (PairingCode, String) = if pairing_code.starts_with('{') {
        // JSON aus QR-Code - URL aus JSON verwenden
        let parsed: PairingCode = serde_json::from_str(pairing_code)?;
        let url = parsed.docflow_url.clone();
        (parsed, url)
    } else if pairing_code.contains('-') {
        // Manueller Code: XXXX-XXXX-XXXX
        // Benutzer-URL verwenden (mit korrektem Port!)
        let url = docflow_url.ok_or("DocFlow-URL wird für manuelle Codes benötigt")?;
        let resolved = resolve_manual_code(pairing_code, url).await?;
        // Benutzer-URL hat Priorität (Server-Antwort könnte Port fehlen durch Reverse-Proxy)
        (resolved, url.trim_end_matches('/').to_string())
    } else {
        return Err("Ungültiger Pairing-Code".into());
    };

    // Bridge bei DocFlow registrieren (mit effektiver URL inkl. korrektem Port)
    let client = reqwest::Client::new();
    let register_url = format!("{}/api/scanner/bridge/register", effective_url);

    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "Unknown".to_string());

    let request = RegisterRequest {
        pairing_token: code.pairing_token,
        bridge_name: code.bridge_name.unwrap_or_else(|| format!("Bridge auf {}", hostname)),
        bridge_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        hostname,
    };

    let response = client
        .post(&register_url)
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("Registrierung fehlgeschlagen: {}", error_text).into());
    }

    let mut result: PairingResult = response.json().await?;
    // Effektive URL speichern (mit korrektem Port!)
    result.docflow_url = effective_url.clone();

    // API-Key sicher speichern (Keyring)
    if let Ok(entry) = keyring::Entry::new("docflow-scanner-bridge", "api_key") {
        let _ = entry.set_password(&result.api_key);
    }

    // DocFlow-URL speichern (mit korrektem Port)
    if let Ok(entry) = keyring::Entry::new("docflow-scanner-bridge", "docflow_url") {
        let _ = entry.set_password(&effective_url);
    }

    Ok(result)
}

/// Löst manuellen Pairing-Code auf
async fn resolve_manual_code(code: &str, docflow_url: &str) -> Result<PairingCode, Box<dyn std::error::Error + Send + Sync>> {
    // DocFlow URL vom Parameter verwenden (z.B. "http://localhost:4000")
    let resolve_url = format!("{}/api/scanner/bridge/resolve-code", docflow_url.trim_end_matches('/'));

    let client = reqwest::Client::new();
    let response = client
        .post(&resolve_url)
        .json(&serde_json::json!({ "code": code }))
        .send()
        .await
        .map_err(|e| format!("Verbindung zu {} fehlgeschlagen: {}", resolve_url, e))?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("Code-Auflösung fehlgeschlagen: {}", error_text).into());
    }

    Ok(response.json().await?)
}

/// Lädt gespeicherte Verbindungsdaten
pub async fn load_saved_connection() -> Option<(String, String)> {
    let api_key = keyring::Entry::new("docflow-scanner-bridge", "api_key")
        .ok()?
        .get_password()
        .ok()?;

    let docflow_url = keyring::Entry::new("docflow-scanner-bridge", "docflow_url")
        .ok()?
        .get_password()
        .ok()?;

    Some((api_key, docflow_url))
}

/// Validiert bestehende Verbindung
pub async fn validate_connection(api_key: &str, docflow_url: &str) -> bool {
    let client = reqwest::Client::new();
    let status_url = format!("{}/api/scanner/bridge/status", docflow_url);

    let response = client
        .get(&status_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await;

    response.map(|r| r.status().is_success()).unwrap_or(false)
}
