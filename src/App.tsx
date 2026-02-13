import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Scan, Link2, Settings, RefreshCw, CheckCircle, XCircle, Loader2 } from 'lucide-react';
import './App.css';

interface BridgeStatus {
  connected: boolean;
  docflow_url: string | null;
  scanner_count: number;
  last_discovery: string | null;
  version: string;
}

interface Scanner {
  id: string;
  name: string;
  manufacturer: string;
  model: string;
  ip: string;
  port: number;
  protocols: string[];
  discovery_method: string;
}

type View = 'status' | 'pairing' | 'scanners' | 'settings';

function App() {
  const [view, setView] = useState<View>('status');
  const [status, setStatus] = useState<BridgeStatus | null>(null);
  const [scanners, setScanners] = useState<Scanner[]>([]);
  const [pairingCode, setPairingCode] = useState('');
  const [docflowUrl, setDocflowUrl] = useState('http://localhost:4000');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  // Status beim Start laden
  useEffect(() => {
    loadStatus();
  }, []);

  const loadStatus = async () => {
    try {
      const s = await invoke<BridgeStatus>('get_status');
      setStatus(s);
      if (!s.connected) {
        setView('pairing');
      }
    } catch (e) {
      console.error('Status laden fehlgeschlagen:', e);
    }
  };

  const discoverScanners = async () => {
    setLoading(true);
    setError('');
    try {
      const found = await invoke<Scanner[]>('discover_scanners');
      setScanners(found);
      setView('scanners');
      await loadStatus();
    } catch (e) {
      setError(`Scanner-Suche fehlgeschlagen: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  const handlePairing = async () => {
    if (!pairingCode.trim()) {
      setError('Bitte Pairing-Code eingeben');
      return;
    }
    if (!docflowUrl.trim()) {
      setError('Bitte DocFlow-URL eingeben');
      return;
    }
    setLoading(true);
    setError('');
    try {
      // DocFlow-URL nur bei manuellen Codes (XXXX-XXXX-XXXX) übergeben
      // Bei QR-Code/JSON ist die URL bereits enthalten
      const isManualCode = !pairingCode.trim().startsWith('{');
      await invoke('pair_with_docflow', {
        pairingCode: pairingCode.trim(),
        docflowUrl: isManualCode ? docflowUrl.trim() : null
      });
      await loadStatus();
      setView('status');
      setPairingCode('');
    } catch (e) {
      setError(`Verbindung fehlgeschlagen: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  const handleDisconnect = async () => {
    try {
      await invoke('disconnect');
      await loadStatus();
      setView('pairing');
    } catch (e) {
      setError(`Trennen fehlgeschlagen: ${e}`);
    }
  };

  return (
    <div className="app">
      {/* Header */}
      <header className="header">
        <div className="header-logo">
          <Scan className="logo-icon" />
          <div>
            <h1>DocFlow Scanner Bridge</h1>
            <span className="version">v{status?.version || '1.0.0'}</span>
          </div>
        </div>
        <div className={`status-badge ${status?.connected ? 'connected' : 'disconnected'}`}>
          {status?.connected ? (
            <>
              <CheckCircle size={14} />
              Verbunden
            </>
          ) : (
            <>
              <XCircle size={14} />
              Nicht verbunden
            </>
          )}
        </div>
      </header>

      {/* Navigation */}
      <nav className="nav">
        <button
          className={view === 'status' ? 'active' : ''}
          onClick={() => setView('status')}
        >
          Status
        </button>
        <button
          className={view === 'scanners' ? 'active' : ''}
          onClick={() => setView('scanners')}
        >
          Scanner ({status?.scanner_count || 0})
        </button>
        <button
          className={view === 'pairing' ? 'active' : ''}
          onClick={() => setView('pairing')}
        >
          Verbindung
        </button>
        <button
          className={view === 'settings' ? 'active' : ''}
          onClick={() => setView('settings')}
        >
          <Settings size={16} />
        </button>
      </nav>

      {/* Fehler-Anzeige */}
      {error && (
        <div className="error-banner">
          {error}
          <button onClick={() => setError('')}>×</button>
        </div>
      )}

      {/* Content */}
      <main className="content">
        {view === 'status' && (
          <div className="view-status">
            <div className="status-card">
              <h2>Bridge-Status</h2>
              {status?.connected ? (
                <div className="status-info">
                  <div className="info-row">
                    <span>DocFlow Server:</span>
                    <span>{status.docflow_url}</span>
                  </div>
                  <div className="info-row">
                    <span>Erkannte Scanner:</span>
                    <span>{status.scanner_count}</span>
                  </div>
                  <div className="info-row">
                    <span>Letzte Suche:</span>
                    <span>
                      {status.last_discovery
                        ? new Date(status.last_discovery).toLocaleString('de-DE')
                        : 'Noch nicht gesucht'}
                    </span>
                  </div>
                </div>
              ) : (
                <p className="not-connected">
                  Nicht mit DocFlow verbunden. Bitte unter "Verbindung" den Pairing-Code eingeben.
                </p>
              )}
            </div>

            <button
              className="btn-primary btn-large"
              onClick={discoverScanners}
              disabled={loading}
            >
              {loading ? (
                <>
                  <Loader2 className="spin" size={20} />
                  Suche läuft...
                </>
              ) : (
                <>
                  <RefreshCw size={20} />
                  Scanner suchen
                </>
              )}
            </button>
          </div>
        )}

        {view === 'scanners' && (
          <div className="view-scanners">
            <div className="scanners-header">
              <h2>Gefundene Scanner</h2>
              <button className="btn-icon" onClick={discoverScanners} disabled={loading}>
                <RefreshCw size={18} className={loading ? 'spin' : ''} />
              </button>
            </div>

            {scanners.length === 0 ? (
              <div className="no-scanners">
                <Scan size={48} />
                <p>Keine Scanner gefunden</p>
                <p className="hint">
                  Stellen Sie sicher, dass Ihre Scanner eingeschaltet und im Netzwerk erreichbar sind.
                </p>
              </div>
            ) : (
              <div className="scanner-list">
                {scanners.map((scanner) => (
                  <div key={scanner.id} className="scanner-card">
                    <div className="scanner-icon">
                      <Scan size={24} />
                    </div>
                    <div className="scanner-info">
                      <h3>{scanner.name}</h3>
                      <p className="scanner-details">
                        {scanner.manufacturer} • {scanner.ip}:{scanner.port}
                      </p>
                      <div className="scanner-tags">
                        {scanner.protocols.map((p) => (
                          <span key={p} className="tag">
                            {p.toUpperCase()}
                          </span>
                        ))}
                        <span className="tag tag-method">{scanner.discovery_method}</span>
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {view === 'pairing' && (
          <div className="view-pairing">
            <div className="pairing-card">
              <Link2 size={48} className="pairing-icon" />
              <h2>Mit DocFlow verbinden</h2>

              {status?.connected ? (
                <div className="connected-info">
                  <p>
                    <CheckCircle size={18} className="success-icon" />
                    Verbunden mit: <strong>{status.docflow_url}</strong>
                  </p>
                  <button className="btn-danger" onClick={handleDisconnect}>
                    Verbindung trennen
                  </button>
                </div>
              ) : (
                <>
                  <p className="pairing-hint">
                    Öffnen Sie DocFlow → Einstellungen → Scanner → "Bridge verbinden"
                    und geben Sie den angezeigten Code hier ein:
                  </p>

                  <div className="input-group">
                    <label htmlFor="docflow-url">DocFlow Server URL:</label>
                    <input
                      id="docflow-url"
                      type="text"
                      value={docflowUrl}
                      onChange={(e) => setDocflowUrl(e.target.value)}
                      placeholder="http://localhost:4000"
                      className="pairing-input"
                    />
                    <span className="input-hint">
                      Lokaler Docker: http://localhost:4000 | Cloud: https://docflow.example.de
                    </span>
                  </div>

                  <div className="input-group">
                    <label htmlFor="pairing-code">Pairing-Code:</label>
                    <input
                      id="pairing-code"
                      type="text"
                      value={pairingCode}
                      onChange={(e) => setPairingCode(e.target.value)}
                      placeholder="XXXX-XXXX-XXXX"
                      className="pairing-input"
                    />
                  </div>

                  <button
                    className="btn-primary"
                    onClick={handlePairing}
                    disabled={loading || !pairingCode.trim() || !docflowUrl.trim()}
                  >
                    {loading ? (
                      <>
                        <Loader2 className="spin" size={18} />
                        Verbinde...
                      </>
                    ) : (
                      'Verbinden'
                    )}
                  </button>
                </>
              )}
            </div>
          </div>
        )}

        {view === 'settings' && (
          <div className="view-settings">
            <h2>Einstellungen</h2>

            <div className="settings-section">
              <h3>Autostart</h3>
              <label className="toggle">
                <input type="checkbox" defaultChecked />
                <span>Bei Systemstart automatisch starten</span>
              </label>
            </div>

            <div className="settings-section">
              <h3>Benachrichtigungen</h3>
              <label className="toggle">
                <input type="checkbox" defaultChecked />
                <span>Bei neuen Scannern benachrichtigen</span>
              </label>
            </div>

            <div className="settings-section">
              <h3>Über</h3>
              <p>DocFlow Scanner Bridge v{status?.version}</p>
              <p className="hint">© 2026 OneMillion Digital UG</p>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
