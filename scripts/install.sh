#!/usr/bin/env bash
# jinx installer — macOS & Linux
# Usage: curl -fsSL https://raw.githubusercontent.com/ramtoearth/jinx/main/install.sh | bash
set -euo pipefail

REPO="ramtoearth/jinx"
INSTALL_DIR="${JINX_INSTALL_DIR:-$HOME/.local/bin}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# ── Detect platform ──────────────────────────────────────────────────────────
OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
  Darwin)
    case "$ARCH" in
      arm64)  TARGET="aarch64-apple-darwin" ;;
      x86_64) TARGET="x86_64-apple-darwin" ;;
      *)      echo "Error: unsupported architecture on macOS: $ARCH"; exit 1 ;;
    esac
    SHASUM="shasum -a 256"
    ;;
  Linux)
    case "$ARCH" in
      x86_64)        TARGET="x86_64-unknown-linux-gnu" ;;
      aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
      *)             echo "Error: unsupported architecture on Linux: $ARCH"; exit 1 ;;
    esac
    SHASUM="sha256sum"
    ;;
  *)
    echo "Error: unsupported operating system: $OS"
    echo "On Windows run install.ps1 — see the README."
    exit 1
    ;;
esac

# ── Resolve latest release ───────────────────────────────────────────────────
echo "→ Looking up the latest jinx release..."
VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": "\(.*\)".*/\1/')

if [ -z "$VERSION" ]; then
  echo "Error: could not determine the latest version."
  exit 1
fi
echo "  Version: $VERSION"

# ── Download archive + checksum ──────────────────────────────────────────────
ARCHIVE="jinx-${VERSION}-${TARGET}.tar.gz"
BASE_URL="https://github.com/$REPO/releases/download/$VERSION"

echo "→ Downloading $ARCHIVE..."
curl -fsSL "$BASE_URL/$ARCHIVE"         -o "$TMP/$ARCHIVE"
curl -fsSL "$BASE_URL/$ARCHIVE.sha256"  -o "$TMP/$ARCHIVE.sha256"

# ── Verify integrity ─────────────────────────────────────────────────────────
echo "→ Verifying integrity (SHA-256)..."
(cd "$TMP" && $SHASUM -c "$ARCHIVE.sha256")

# ── Install binary ───────────────────────────────────────────────────────────
echo "→ Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
tar -xzf "$TMP/$ARCHIVE" -C "$INSTALL_DIR"
chmod +x "$INSTALL_DIR/jinx"

# ── Install uv if missing ────────────────────────────────────────────────────
if ! command -v uv &>/dev/null; then
  echo "→ Installing uv (Python environment manager)..."
  curl -LsSf https://astral.sh/uv/install.sh | sh
  export PATH="$HOME/.local/bin:$PATH"
  echo "  uv installed."
else
  echo "→ uv is already installed ($(uv --version))."
fi

# ── PATH hint ────────────────────────────────────────────────────────────────
if ! printf ':%s:' "$PATH" | grep -q ":$INSTALL_DIR:"; then
  echo ""
  echo "⚠  $INSTALL_DIR is not in your PATH."
  echo "   Add this line to ~/.bashrc, ~/.zshrc or ~/.profile:"
  echo ""
  echo "     export PATH=\"$INSTALL_DIR:\$PATH\""
  echo ""
fi

echo "✓ jinx $VERSION installed."
echo ""
echo "Next steps:"
echo "  1. Install Ollama:          https://ollama.com"
echo "  2. Pull a model:            ollama pull llama3.2:3b"
echo "  3. Start the app:           jinx"
