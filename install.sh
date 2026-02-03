#!/bin/bash
set -e

# Forge CLI Installer (Rust Edition)
# Usage: curl -fsSL https://forge.dev/install-rs.sh | bash

CDN_BASE="https://pub-f7e4afbb09c44b6cb5f35c4d5d60af58.r2.dev/forge-rs"
INSTALL_DIR="/usr/local/bin"

# Detect OS and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  darwin) 
    case "$ARCH" in
      x86_64) TARGET="x86_64-apple-darwin" ;;
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  linux)
    case "$ARCH" in
      x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
      aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  mingw*|msys*|cygwin*)
    echo "For Windows, download from: ${CDN_BASE}/latest/forge-x86_64-pc-windows-msvc.zip"
    exit 1
    ;;
  *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

echo "🔨 Installing Forge CLI (Rust Edition)..."
echo "  OS: $OS"
echo "  Arch: $ARCH"
echo "  Target: $TARGET"

# Get latest version
VERSION=$(curl -fsSL "${CDN_BASE}/latest/version.txt" 2>/dev/null || echo "")

if [ -z "$VERSION" ]; then
  echo "  Version: latest"
  VERSION_PATH="latest"
else
  echo "  Version: $VERSION"
  VERSION_PATH="v${VERSION}"
fi

# Download binary
ARCHIVE_NAME="forge-${TARGET}.tar.xz"
DOWNLOAD_URL="${CDN_BASE}/${VERSION_PATH}/${ARCHIVE_NAME}"
echo "  Downloading: $ARCHIVE_NAME"

TMP_DIR=$(mktemp -d)
trap "rm -rf $TMP_DIR" EXIT

if ! curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/forge.tar.xz"; then
  echo "Error: Failed to download from $DOWNLOAD_URL"
  echo ""
  echo "Try installing from GitHub instead:"
  echo "  gh release download --repo tharunmarella/forge-rs --pattern '*${TARGET}*'"
  exit 1
fi

# Extract (tar.xz)
cd "$TMP_DIR"
tar -xJf forge.tar.xz

# Find the binary (it's in a subdirectory)
BINARY=$(find . -name "forge" -type f -perm +111 2>/dev/null | head -1)
if [ -z "$BINARY" ]; then
  BINARY=$(find . -name "forge" -type f | head -1)
fi

if [ -z "$BINARY" ]; then
  echo "Error: Could not find forge binary in archive"
  exit 1
fi

# Install
if [ -w "$INSTALL_DIR" ]; then
  mv "$BINARY" "$INSTALL_DIR/forge"
else
  echo "  Installing to $INSTALL_DIR (requires sudo)"
  sudo mv "$BINARY" "$INSTALL_DIR/forge"
fi

chmod +x "$INSTALL_DIR/forge"

echo ""
echo "✅ Forge CLI installed successfully!"
echo ""
forge --version 2>/dev/null || true
echo ""
echo "Get started:"
echo "  forge setup     # Configure API key"
echo "  forge           # Start interactive mode"
echo "  forge --help    # Show all options"
echo ""
echo "For semantic search, install local embeddings:"
echo "  ollama pull nomic-embed-text"
