# DocFlow Scanner Bridge - Multi-Platform Build
# Baut die Bridge für Linux (AppImage/deb) und Windows (exe/msi)

FROM rust:1.75-bookworm AS rust-base

# Grundlegende Build-Tools
RUN apt-get update && apt-get install -y \
    build-essential \
    curl \
    wget \
    file \
    libssl-dev \
    libgtk-3-dev \
    libwebkit2gtk-4.1-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    libsoup-3.0-dev \
    libjavascriptcoregtk-4.1-dev \
    && rm -rf /var/lib/apt/lists/*

# Node.js installieren
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs

# Tauri CLI installieren
RUN cargo install tauri-cli

WORKDIR /app

# ============================================
# Stage: Linux Build (AppImage + deb)
# ============================================
FROM rust-base AS linux-builder

# Abhängigkeiten kopieren und installieren
COPY package.json package-lock.json ./
RUN npm ci

# Rust-Abhängigkeiten vorab bauen (Cache)
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./src-tauri/
RUN mkdir -p src-tauri/src && echo "fn main() {}" > src-tauri/src/main.rs
WORKDIR /app/src-tauri
RUN cargo build --release || true
WORKDIR /app

# Quellcode kopieren
COPY . .

# Frontend bauen
RUN npm run build

# Tauri für Linux bauen
WORKDIR /app/src-tauri
RUN cargo tauri build --bundles appimage,deb

# ============================================
# Stage: Windows Build (Cross-Compilation)
# ============================================
FROM rust-base AS windows-builder

# Windows Cross-Compilation Tools
RUN apt-get update && apt-get install -y \
    mingw-w64 \
    nsis \
    && rm -rf /var/lib/apt/lists/*

# Windows Target hinzufügen
RUN rustup target add x86_64-pc-windows-gnu

# Abhängigkeiten kopieren
COPY package.json package-lock.json ./
RUN npm ci

COPY . .

# Frontend bauen
RUN npm run build

# Windows Build (Cross-Compile)
WORKDIR /app/src-tauri
ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
RUN cargo build --release --target x86_64-pc-windows-gnu || echo "Windows cross-compile may need additional setup"

# ============================================
# Stage: Output - Fertige Binaries sammeln
# ============================================
FROM alpine:latest AS output

RUN mkdir -p /output/linux /output/windows

# Linux Artifacts kopieren
COPY --from=linux-builder /app/src-tauri/target/release/bundle/appimage/*.AppImage /output/linux/ 2>/dev/null || true
COPY --from=linux-builder /app/src-tauri/target/release/bundle/deb/*.deb /output/linux/ 2>/dev/null || true

# Windows Artifacts kopieren (falls erfolgreich)
COPY --from=windows-builder /app/src-tauri/target/x86_64-pc-windows-gnu/release/*.exe /output/windows/ 2>/dev/null || true

CMD ["ls", "-la", "/output"]
