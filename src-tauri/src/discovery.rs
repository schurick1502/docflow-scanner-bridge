// Scanner Discovery - Automatische Erkennung von Scannern im Netzwerk
// Unterstützt: mDNS/Bonjour (eSCL), WSD, IP-Range Scan

use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;
use tokio::time::timeout;

/// Gefundener Scanner
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredScanner {
    pub id: String,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    pub ip: String,
    pub port: u16,
    pub use_tls: bool,
    pub protocols: Vec<String>,
    pub capabilities: ScannerCapabilities,
    pub discovery_method: String,
}

/// Scanner-Fähigkeiten
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannerCapabilities {
    pub duplex: bool,
    pub adf: bool,
    pub flatbed: bool,
    pub max_resolution: u32,
    pub color_modes: Vec<String>,
    pub formats: Vec<String>,
}

/// Service-Typen für mDNS Discovery (Reihenfolge = Priorität: eSCL bevorzugt über IPP)
const MDNS_SERVICE_TYPES: &[&str] = &[
    "_uscan._tcp.local.",   // eSCL Scanner (HTTP) — höchste Priorität
    "_uscans._tcp.local.",  // eSCL Scanner (HTTPS)
    "_ipp._tcp.local.",     // IPP Printer/Scanner — nur als Fallback
    "_scanner._tcp.local.", // Generic Scanner
];

/// Führt alle Discovery-Methoden aus
pub async fn discover_all() -> Result<Vec<DiscoveredScanner>, Box<dyn std::error::Error + Send + Sync>> {
    let mut all_scanners = HashMap::new();

    // 1. mDNS Discovery (primär)
    if let Ok(mdns_scanners) = discover_mdns().await {
        for scanner in mdns_scanners {
            all_scanners.insert(scanner.ip.clone(), scanner);
        }
    }

    // 2. IP-Range Scan (Fallback wenn mDNS nichts findet)
    if all_scanners.is_empty() {
        if let Ok(ip_scanners) = discover_ip_range().await {
            for scanner in ip_scanners {
                all_scanners.entry(scanner.ip.clone()).or_insert(scanner);
            }
        }
    }

    Ok(all_scanners.into_values().collect())
}

/// mDNS/Bonjour Discovery für eSCL-Scanner
async fn discover_mdns() -> Result<Vec<DiscoveredScanner>, Box<dyn std::error::Error + Send + Sync>> {
    let mdns = ServiceDaemon::new()?;
    let mut scanners: HashMap<String, DiscoveredScanner> = HashMap::new();
    // Merken welche Scanner via eSCL (nicht IPP) gefunden wurden
    let mut escl_ips: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Receiver für alle Service-Typen
    for service_type in MDNS_SERVICE_TYPES {
        let is_escl = service_type.starts_with("_uscan");
        let is_escl_tls = *service_type == "_uscans._tcp.local.";
        let receiver = mdns.browse(service_type)?;

        // 5 Sekunden Discovery-Zeit
        let discovery_task = async {
            loop {
                match receiver.recv_async().await {
                    Ok(event) => {
                        if let ServiceEvent::ServiceResolved(info) = event {
                            if let Some(mut scanner) = parse_mdns_service(&info) {
                                if is_escl_tls {
                                    scanner.use_tls = true;
                                }
                                let ip = scanner.ip.clone();
                                if is_escl {
                                    // eSCL-Fund: immer eintragen
                                    escl_ips.insert(ip.clone());
                                    scanners.insert(ip, scanner);
                                } else if !escl_ips.contains(&ip) {
                                    // IPP/Generic: nur eintragen wenn kein eSCL-Fund für diese IP
                                    scanners.insert(ip, scanner);
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        };

        let _ = timeout(Duration::from_secs(5), discovery_task).await;
    }

    mdns.shutdown()?;
    Ok(scanners.into_values().collect())
}

/// Parst mDNS ServiceInfo zu DiscoveredScanner
fn parse_mdns_service(info: &mdns_sd::ServiceInfo) -> Option<DiscoveredScanner> {
    let addresses: Vec<_> = info.get_addresses().iter().collect();
    let ip = addresses.first()?.to_string();
    let port = info.get_port();

    // TXT-Records parsen
    let properties = info.get_properties();
    let model = properties
        .get("ty")
        .or_else(|| properties.get("product"))
        .map(|v| v.val_str().to_string())
        .unwrap_or_else(|| info.get_fullname().to_string());

    let uuid = properties
        .get("uuid")
        .map(|v| v.val_str().to_string())
        .unwrap_or_else(|| format!("{}:{}", ip, port));

    let manufacturer = extract_manufacturer(&model);

    // Capabilities aus TXT-Records
    let duplex = properties
        .get("duplex")
        .map(|v| v.val_str().to_lowercase())
        .map(|v| v == "t" || v == "true" || v == "1")
        .unwrap_or(false);

    let input_sources = properties
        .get("is")
        .map(|v| v.val_str().to_lowercase())
        .unwrap_or_default();
    let adf = input_sources.contains("adf") || input_sources.contains("feeder");
    let flatbed = input_sources.contains("platen") || input_sources.is_empty();

    Some(DiscoveredScanner {
        id: uuid,
        name: model.clone(),
        manufacturer,
        model,
        ip,
        port,
        use_tls: false, // Wird ggf. vom Caller auf true gesetzt (_uscans._tcp)
        protocols: vec!["escl".to_string()],
        capabilities: ScannerCapabilities {
            duplex,
            adf,
            flatbed,
            max_resolution: 600,
            color_modes: vec!["RGB24".to_string(), "Grayscale8".to_string()],
            formats: vec!["application/pdf".to_string(), "image/jpeg".to_string()],
        },
        discovery_method: "mdns".to_string(),
    })
}

/// IP-Range Scan für Scanner ohne mDNS
async fn discover_ip_range() -> Result<Vec<DiscoveredScanner>, Box<dyn std::error::Error + Send + Sync>> {
    let mut scanners = Vec::new();

    // Lokales Netzwerk ermitteln
    let local_ip = local_ip_address::local_ip()?;
    let subnet = get_subnet(&local_ip);

    // Ports für eSCL Scanner
    let ports = [80, 443, 8080, 9100];

    // Parallel alle IPs im Subnet scannen
    let mut tasks = Vec::new();
    for i in 1..=254 {
        let ip = format!("{}.{}", subnet, i);
        for &port in &ports {
            let ip_clone = ip.clone();
            tasks.push(tokio::spawn(async move {
                probe_escl_endpoint(&ip_clone, port).await
            }));
        }
    }

    // Ergebnisse sammeln (mit Timeout)
    for task in tasks {
        if let Ok(Ok(Some(scanner))) = timeout(Duration::from_secs(30), task).await {
            scanners.push(scanner);
        }
    }

    Ok(scanners)
}

/// Prüft ob unter IP:Port ein eSCL-Endpunkt erreichbar ist
async fn probe_escl_endpoint(ip: &str, port: u16) -> Option<DiscoveredScanner> {
    let scheme = if port == 443 { "https" } else { "http" };
    let url = format!("{}://{}:{}/eSCL/ScannerCapabilities", scheme, ip, port);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .danger_accept_invalid_certs(true)
        .build()
        .ok()?;

    let response = client.get(&url).send().await.ok()?;

    if response.status().is_success() {
        let content = response.text().await.ok()?;

        // Prüfen ob es eSCL XML ist
        if content.contains("ScannerCapabilities") {
            return Some(DiscoveredScanner {
                id: format!("{}:{}", ip, port),
                name: format!("Scanner at {}", ip),
                manufacturer: "Unknown".to_string(),
                model: format!("eSCL Scanner ({})", ip),
                ip: ip.to_string(),
                port,
                use_tls: port == 443,
                protocols: vec!["escl".to_string()],
                capabilities: ScannerCapabilities::default(),
                discovery_method: "ip_scan".to_string(),
            });
        }
    }

    None
}

/// Extrahiert Hersteller aus Modellname
fn extract_manufacturer(model: &str) -> String {
    let model_lower = model.to_lowercase();
    let manufacturers = [
        ("hp", "HP"),
        ("hewlett", "HP"),
        ("canon", "Canon"),
        ("brother", "Brother"),
        ("epson", "Epson"),
        ("samsung", "Samsung"),
        ("xerox", "Xerox"),
        ("lexmark", "Lexmark"),
        ("ricoh", "Ricoh"),
        ("kyocera", "Kyocera"),
        ("konica", "Konica Minolta"),
    ];

    for (key, name) in manufacturers {
        if model_lower.contains(key) {
            return name.to_string();
        }
    }

    "Unknown".to_string()
}

/// Ermittelt Subnet-Prefix aus IP-Adresse
fn get_subnet(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            format!("{}.{}.{}", octets[0], octets[1], octets[2])
        }
        IpAddr::V6(_) => "192.168.1".to_string(), // Fallback für IPv6
    }
}

#[cfg(target_os = "windows")]
pub mod native {
    //! Windows-spezifische Scanner-Erkennung via WIA
    use super::*;

    /// Entdeckt lokale Scanner via Windows Image Acquisition (WIA)
    pub async fn discover_wia() -> Result<Vec<DiscoveredScanner>, Box<dyn std::error::Error + Send + Sync>> {
        // WIA-Implementation für USB-Scanner
        // Erfordert windows-rs crate
        Ok(vec![])
    }
}

#[cfg(target_os = "linux")]
pub mod native {
    //! Linux-spezifische Scanner-Erkennung via SANE
    use super::*;

    /// Entdeckt lokale Scanner via SANE
    pub async fn discover_sane() -> Result<Vec<DiscoveredScanner>, Box<dyn std::error::Error + Send + Sync>> {
        // SANE-Implementation
        // Erfordert libsane
        Ok(vec![])
    }
}

#[cfg(target_os = "macos")]
pub mod native {
    //! macOS-spezifische Scanner-Erkennung via ImageCaptureCore
    use super::*;

    /// Entdeckt lokale Scanner via ImageCaptureCore
    pub async fn discover_image_capture() -> Result<Vec<DiscoveredScanner>, Box<dyn std::error::Error + Send + Sync>> {
        // ImageCaptureCore-Implementation
        Ok(vec![])
    }
}
