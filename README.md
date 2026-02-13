# DocFlow Scanner Bridge

[![Release](https://img.shields.io/github/v/release/schurick1502/docflow-scanner-bridge)](https://github.com/schurick1502/docflow-scanner-bridge/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Desktop-Anwendung zur Verbindung lokaler Netzwerk-Scanner mit [DocFlow](https://docflow.onemillion-digital.de) (Docker/Cloud).

## Download & Installation

### Windows (empfohlen: MSI-Installer)

1. [DocFlow-Scanner-Bridge.msi herunterladen](https://github.com/schurick1502/docflow-scanner-bridge/releases/latest)
2. Doppelklick auf die .msi-Datei
3. Falls Windows SmartScreen eine Warnung zeigt: auf **"Weitere Informationen"** und dann **"Trotzdem ausfuehren"** klicken (die App ist sicher, aber noch nicht mit einem EV-Zertifikat signiert)
4. Installation abschliessen — die Bridge startet automatisch im System Tray

### macOS

1. [DocFlow-Scanner-Bridge.app herunterladen](https://github.com/schurick1502/docflow-scanner-bridge/releases/latest) (Intel oder Apple Silicon)
2. Entpacken und in den Programme-Ordner verschieben
3. Beim ersten Start: Rechtsklick > "Oeffnen" (Gatekeeper-Warnung bestaetigen)

### Linux

**AppImage (alle Distributionen):**
```bash
wget https://github.com/schurick1502/docflow-scanner-bridge/releases/latest/download/docflow-scanner-bridge_amd64.AppImage
chmod +x docflow-scanner-bridge_amd64.AppImage
./docflow-scanner-bridge_amd64.AppImage
```

**Debian/Ubuntu:**
```bash
wget https://github.com/schurick1502/docflow-scanner-bridge/releases/latest/download/docflow-scanner-bridge_amd64.deb
sudo dpkg -i docflow-scanner-bridge_amd64.deb
```

## Erste Schritte (Pairing)

1. **DocFlow oeffnen** und zu Einstellungen > Scanner navigieren
2. **"Bridge verbinden"** klicken — ein Pairing-Code wird angezeigt
3. **Scanner Bridge App** oeffnen und den Code eingeben (oder QR-Code scannen)
4. Die Bridge erkennt automatisch alle Scanner im Netzwerk
5. Scans koennen jetzt direkt aus DocFlow gestartet werden

## Features

- Automatische Scanner-Erkennung (mDNS/Zeroconf, WSD, IP-Scan)
- Einfaches Pairing mit DocFlow via QR-Code oder manuellem Code
- System Tray — laeuft unauffaellig im Hintergrund
- Auto-Update — aktualisiert sich automatisch ueber GitHub Releases
- Cross-Platform — Windows, macOS (Intel + ARM), Linux

## Architektur

```
+------------------------------------------------------------+
|                    DocFlow Scanner Bridge                   |
|  +------------------------------------------------------+  |
|  |  Tauri/Rust Backend                                  |  |
|  |  - System Tray, Auto-Updater, Keyring                |  |
|  |  - Scanner-Zugriff (eSCL, WIA, SANE, ImageCapture)   |  |
|  +------------------------------------------------------+  |
|  +------------------------------------------------------+  |
|  |  React Frontend (WebView)                            |  |
|  |  - Pairing Wizard, Scanner-Liste, Einstellungen      |  |
|  +------------------------------------------------------+  |
|  +------------------------------------------------------+  |
|  |  Discovery Service                                   |  |
|  |  - mDNS/Zeroconf, WS-Discovery, IP-Range Scan       |  |
|  +------------------------------------------------------+  |
+------------------------------------------------------------+
              |                           |
              v                           v
     +----------------+         +--------------------+
     | Lokale Scanner |         |  DocFlow Backend   |
     | (Netzwerk/USB) |         |  (Docker/Cloud)    |
     +----------------+         +--------------------+
```

## Unterstuetzte Scanner-Protokolle

| Protokoll | Windows | macOS | Linux |
|-----------|---------|-------|-------|
| eSCL/AirPrint | Ja | Ja | Ja |
| WSD | Ja | - | - |
| WIA 2.0 | Ja | - | - |
| ImageCapture | - | Ja | - |
| SANE | - | - | Ja |

## Entwicklung

### Voraussetzungen

- [Rust](https://rustup.rs/) 1.70+
- [Node.js](https://nodejs.org/) 18+
- Plattform-spezifisch:
  - **Windows:** MSVC Build Tools
  - **macOS:** Xcode Command Line Tools
  - **Linux:** `sudo apt install libwebkit2gtk-4.1-dev libsane-dev libappindicator3-dev`

### Setup

```bash
git clone https://github.com/schurick1502/docflow-scanner-bridge.git
cd docflow-scanner-bridge
npm install
npm run tauri dev
```

### Release erstellen

```bash
# Version in package.json und src-tauri/tauri.conf.json erhoehen
git tag v1.0.1
git push origin v1.0.1
# GitHub Actions baut automatisch fuer alle Plattformen
```

## API-Kommunikation mit DocFlow

Die Bridge kommuniziert mit dem DocFlow-Backend ueber REST:

```
POST /api/scanner/bridge/register       - Bridge registrieren (Pairing)
POST /api/scanner/bridge/resolve-code   - Pairing-Code aufloesen
GET  /api/scanner/bridge/status         - Verbindungsstatus pruefen
POST /api/scanner/bridge/scanners       - Erkannte Scanner melden
GET  /api/scanner/bridge/pending-scans  - Scan-Jobs abrufen
POST /api/scanner/bridge/scan-upload/{id} - Scan-Ergebnis hochladen
```

## Lizenz

MIT License - siehe [LICENSE](LICENSE)

## Support

- [Issues](https://github.com/schurick1502/docflow-scanner-bridge/issues) — Bugs und Feature-Wuensche
- [DocFlow Dokumentation](https://docflow.onemillion-digital.de) — Hauptanwendung
