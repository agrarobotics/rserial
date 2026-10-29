#!/bin/bash
# Install rserial to the release directory used by the taskbar .desktop entry
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RELEASE_DIR="$HOME/scottsoft_wv/sdn_release_rserial"
BIN_NAME="rserial"

echo "Building release..."
cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"

echo "Installing to $RELEASE_DIR..."
mkdir -p "$RELEASE_DIR"
cp "$SCRIPT_DIR/target/release/$BIN_NAME" "$RELEASE_DIR/$BIN_NAME"

# Ship the HTML help page next to the binary so Ctrl+H can find it
cp "$SCRIPT_DIR/docs_about__rserial/rserial__help.html" "$RELEASE_DIR/rserial__help.html"

echo "Done. Restart the app from your taskbar to use the updated version."
