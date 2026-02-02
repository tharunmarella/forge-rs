#!/bin/bash
set -e

# Forge CLI Installer
# Usage: curl -fsSL https://raw.githubusercontent.com/tharunmarella/forge-rs/master/install.sh | bash

REPO="tharunmarella/forge-rs"
INSTALL_DIR="/usr/local/bin"

# Detect OS and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  darwin) OS="darwin" ;;
  linux) OS="linux" ;;
  *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
  x86_64) ARCH="x64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

BINARY_NAME="forge-${OS}-${ARCH}"

echo "Installing Forge CLI..."
echo "  OS: $OS"
echo "  Arch: $ARCH"

# Get latest release
LATEST=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')

if [ -z "$LATEST" ]; then
  echo "Error: Could not find latest release"
  exit 1
fi

echo "  Version: $LATEST"

# Download binary
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${LATEST}/${BINARY_NAME}.tar.gz"
echo "  Downloading from: $DOWNLOAD_URL"

TMP_DIR=$(mktemp -d)
curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/forge.tar.gz"
tar -xzf "$TMP_DIR/forge.tar.gz" -C "$TMP_DIR"

# Install
if [ -w "$INSTALL_DIR" ]; then
  mv "$TMP_DIR/$BINARY_NAME" "$INSTALL_DIR/forge"
else
  sudo mv "$TMP_DIR/$BINARY_NAME" "$INSTALL_DIR/forge"
fi

chmod +x "$INSTALL_DIR/forge"
rm -rf "$TMP_DIR"

echo ""
echo "Forge CLI installed successfully!"
echo "Run 'forge' to start."
