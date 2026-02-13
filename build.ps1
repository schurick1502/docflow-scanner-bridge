# DocFlow Scanner Bridge - Build Script für Windows
# Baut die Bridge lokal (benötigt Rust) oder via Docker (nur Linux)

param(
    [Parameter()]
    [ValidateSet("local", "docker", "docker-linux", "frontend")]
    [string]$Mode = "docker-linux"
)

$ErrorActionPreference = "Stop"

Write-Host "=== DocFlow Scanner Bridge Build ===" -ForegroundColor Cyan
Write-Host "Mode: $Mode" -ForegroundColor Yellow

switch ($Mode) {
    "local" {
        # Lokaler Build (benötigt Rust)
        Write-Host "`nPrüfe Rust Installation..." -ForegroundColor Green

        $rustVersion = rustc --version 2>&1
        if ($LASTEXITCODE -ne 0) {
            Write-Host "FEHLER: Rust ist nicht installiert!" -ForegroundColor Red
            Write-Host "Installiere Rust von: https://rustup.rs" -ForegroundColor Yellow
            exit 1
        }
        Write-Host "Rust gefunden: $rustVersion" -ForegroundColor Green

        Write-Host "`nInstalliere npm Dependencies..." -ForegroundColor Green
        npm ci

        Write-Host "`nBaue Frontend..." -ForegroundColor Green
        npm run build

        Write-Host "`nBaue Tauri App..." -ForegroundColor Green
        npm run tauri build

        Write-Host "`nFertig! Installer in: src-tauri\target\release\bundle\" -ForegroundColor Cyan
    }

    "docker-linux" {
        # Docker Build für Linux
        Write-Host "`nBaue Linux-Version via Docker..." -ForegroundColor Green
        Write-Host "(Dies dauert beim ersten Mal ~10-15 Minuten)" -ForegroundColor Yellow

        docker compose -f docker-compose.build.yml build build-linux
        docker compose -f docker-compose.build.yml run --rm build-linux

        Write-Host "`nFertig! Linux-Installer in: dist\linux\" -ForegroundColor Cyan
    }

    "frontend" {
        # Nur Frontend bauen (schnell, für Tests)
        Write-Host "`nBaue nur Frontend via Docker..." -ForegroundColor Green

        docker compose -f docker-compose.build.yml run --rm build-frontend

        Write-Host "`nFertig! Frontend in: dist\" -ForegroundColor Cyan
    }

    "docker" {
        # Vollständiger Docker Build (Linux + Windows-Versuch)
        Write-Host "`nBaue alle Plattformen via Docker..." -ForegroundColor Green
        Write-Host "HINWEIS: Windows Cross-Compile ist experimentell" -ForegroundColor Yellow

        docker build -t scanner-bridge-builder .

        # Container erstellen und Artifacts extrahieren
        $containerId = docker create scanner-bridge-builder
        docker cp "${containerId}:/output" ./dist
        docker rm $containerId

        Write-Host "`nFertig! Installer in: dist\" -ForegroundColor Cyan
    }
}

Write-Host "`n=== Build abgeschlossen ===" -ForegroundColor Cyan
