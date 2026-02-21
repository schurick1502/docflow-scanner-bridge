import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Scan, Link2, Settings, RefreshCw, CheckCircle, XCircle, Loader2, FolderSync, FolderOpen, Play, Square } from 'lucide-react';
import './App.css';

interface BridgeStatus {
  connected: boolean;
  docflow_url: string | null;
  scanner_count: number;
  last_discovery: string | null;
  version: string;
  folder_sync_active: boolean;
  folder_sync_path: string | null;
}

interface FolderSyncStatusInfo {
  running: boolean;
  watch_path: string | null;
  files_uploaded: number;
  files_pending: number;
  errors: number;
  last_upload: string | null;
  last_error: string | null;
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

type View = 'status' | 'pairing' | 'scanners' | 'folder_sync' | 'settings';

function App() {
  const [view, setView] = useState<View>('status');
  const [status, setStatus] = useState<BridgeStatus | null>(null);
  const [scanners, setScanners] = useState<Scanner[]>([]);
  const [pairingCode, setPairingCode] = useState('');
  const [docflowUrl, setDocflowUrl] = useState(() => {
    // Letzte erfolgreiche URL aus localStorage laden
    return localStorage.getItem('docflow-bridge-url') || '';
  });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  // Folder Sync State
  const [folderSyncStatus, setFolderSyncStatus] = useState<FolderSyncStatusInfo | null>(null);
  const [watchPath, setWatchPath] = useState(() => localStorage.getItem('docflow-watch-path') || '');
  const [postAction, setPostAction] = useState<string>(() => localStorage.getItem('docflow-post-action') || 'move');

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
      // URL fuer naechsten Start speichern
      if (docflowUrl.trim()) {
        localStorage.setItem('docflow-bridge-url', docflowUrl.trim());
      }
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
      setFolderSyncStatus(null);
      setView('pairing');
    } catch (e) {
      setError(`Trennen fehlgeschlagen: ${e}`);
    }
  };

  // Folder Sync Handler
  const loadFolderSyncStatus = async () => {
    try {
      const s = await invoke<FolderSyncStatusInfo>('get_folder_sync_status');
      setFolderSyncStatus(s);
    } catch (e) {
      console.error('Folder-Sync-Status laden fehlgeschlagen:', e);
    }
  };

  const handlePickFolder = async () => {
    try {
      const path = await invoke<string | null>('pick_folder');
      if (path) {
        setWatchPath(path);
        localStorage.setItem('docflow-watch-path', path);
      }
    } catch (e) {
      setError(`Ordner-Auswahl fehlgeschlagen: ${e}`);
    }
  };

  const handleStartFolderSync = async () => {
    if (!watchPath.trim()) {
      setError('Bitte einen Ordner auswählen');
      return;
    }
    setLoading(true);
    setError('');
    try {
      await invoke('configure_folder_sync', {
        watchPath: watchPath.trim(),
        postAction: postAction,
      });
      localStorage.setItem('docflow-watch-path', watchPath.trim());
      localStorage.setItem('docflow-post-action', postAction);
      await loadStatus();
      await loadFolderSyncStatus();
    } catch (e) {
      setError(`Folder-Sync starten fehlgeschlagen: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  const handleStopFolderSync = async () => {
    try {
      await invoke('stop_folder_sync');
      await loadStatus();
      await loadFolderSyncStatus();
    } catch (e) {
      setError(`Folder-Sync stoppen fehlgeschlagen: ${e}`);
    }
  };

  // Status-Polling für Folder-Sync (alle 2 Sekunden wenn aktiv)
  useEffect(() => {
    if (view === 'folder_sync' || status?.folder_sync_active) {
      loadFolderSyncStatus();
      const interval = setInterval(loadFolderSyncStatus, 2000);
      return () => clearInterval(interval);
    }
  }, [view, status?.folder_sync_active]);

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
          className={view === 'folder_sync' ? 'active' : ''}
          onClick={() => setView('folder_sync')}
        >
          <FolderSync size={16} />
          Ordner-Sync
          {status?.folder_sync_active && <span className="sync-dot" />}
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

            {/* Folder-Sync Zusammenfassung */}
            {status?.folder_sync_active && folderSyncStatus?.running && (
              <div className="status-card sync-summary">
                <div className="sync-summary-header">
                  <FolderSync size={18} />
                  <strong>Ordner-Sync aktiv</strong>
                  <span className="sync-dot" />
                </div>
                <p className="hint">
                  {status.folder_sync_path} — {folderSyncStatus.files_uploaded} Dateien hochgeladen
                </p>
              </div>
            )}

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
                      placeholder="https://docflow.example.de"
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

        {view === 'folder_sync' && (
          <div className="view-folder-sync">
            <h2>Ordner-Sync</h2>
            <p className="hint">
              Überwacht einen lokalen Ordner und lädt neue Dateien automatisch zu DocFlow hoch.
            </p>

            {!status?.connected ? (
              <div className="not-connected">
                <p>Bitte zuerst mit DocFlow verbinden.</p>
              </div>
            ) : (
              <>
                {/* Ordner-Auswahl */}
                <div className="settings-section">
                  <h3>Überwachter Ordner</h3>
                  <div className="folder-picker">
                    <input
                      type="text"
                      value={watchPath}
                      onChange={(e) => setWatchPath(e.target.value)}
                      placeholder="C:\Users\...\Scans"
                      className="pairing-input"
                      disabled={folderSyncStatus?.running}
                    />
                    <button
                      className="btn-icon"
                      onClick={handlePickFolder}
                      disabled={folderSyncStatus?.running}
                      title="Ordner auswählen"
                    >
                      <FolderOpen size={18} />
                    </button>
                  </div>
                </div>

                {/* Post-Upload Aktion */}
                <div className="settings-section">
                  <h3>Nach Upload</h3>
                  <div className="radio-group">
                    <label className="radio-label">
                      <input
                        type="radio"
                        name="postAction"
                        value="move"
                        checked={postAction === 'move'}
                        onChange={() => setPostAction('move')}
                        disabled={folderSyncStatus?.running}
                      />
                      In "uploaded" Unterordner verschieben
                    </label>
                    <label className="radio-label">
                      <input
                        type="radio"
                        name="postAction"
                        value="delete"
                        checked={postAction === 'delete'}
                        onChange={() => setPostAction('delete')}
                        disabled={folderSyncStatus?.running}
                      />
                      Datei löschen
                    </label>
                    <label className="radio-label">
                      <input
                        type="radio"
                        name="postAction"
                        value="keep"
                        checked={postAction === 'keep'}
                        onChange={() => setPostAction('keep')}
                        disabled={folderSyncStatus?.running}
                      />
                      Datei behalten (für Tests)
                    </label>
                  </div>
                </div>

                {/* Start/Stop Button */}
                <div className="sync-controls">
                  {folderSyncStatus?.running ? (
                    <button className="btn-danger btn-large" onClick={handleStopFolderSync}>
                      <Square size={18} />
                      Sync stoppen
                    </button>
                  ) : (
                    <button
                      className="btn-primary btn-large"
                      onClick={handleStartFolderSync}
                      disabled={loading || !watchPath.trim()}
                    >
                      {loading ? (
                        <>
                          <Loader2 className="spin" size={18} />
                          Starte...
                        </>
                      ) : (
                        <>
                          <Play size={18} />
                          Sync starten
                        </>
                      )}
                    </button>
                  )}
                </div>

                {/* Status-Anzeige */}
                {folderSyncStatus && (folderSyncStatus.running || folderSyncStatus.files_uploaded > 0) && (
                  <div className="status-card sync-status-card">
                    <h3>
                      {folderSyncStatus.running && <span className="sync-dot-large" />}
                      Sync-Status
                    </h3>
                    <div className="status-info">
                      <div className="info-row">
                        <span>Status:</span>
                        <span className={folderSyncStatus.running ? 'text-success' : ''}>
                          {folderSyncStatus.running ? 'Aktiv' : 'Gestoppt'}
                        </span>
                      </div>
                      <div className="info-row">
                        <span>Hochgeladen:</span>
                        <span>{folderSyncStatus.files_uploaded} Dateien</span>
                      </div>
                      {folderSyncStatus.errors > 0 && (
                        <div className="info-row">
                          <span>Fehler:</span>
                          <span className="text-error">{folderSyncStatus.errors}</span>
                        </div>
                      )}
                      {folderSyncStatus.last_upload && (
                        <div className="info-row">
                          <span>Letzter Upload:</span>
                          <span>{new Date(folderSyncStatus.last_upload).toLocaleString('de-DE')}</span>
                        </div>
                      )}
                      {folderSyncStatus.last_error && (
                        <div className="info-row">
                          <span>Letzter Fehler:</span>
                          <span className="text-error">{folderSyncStatus.last_error}</span>
                        </div>
                      )}
                    </div>
                  </div>
                )}
              </>
            )}
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
