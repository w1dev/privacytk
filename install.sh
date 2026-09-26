#!/usr/bin/env sh
# privacytk corporate installer.
#
# Unlike the upstream rtk installer (which fetches a precompiled binary from
# `raw.githubusercontent.com/rtk-ai/rtk/...`), this script does NOT download
# a binary over the network. corporate environments ship the binary in their
# base image (see docs/CORPORATE.md §2), so this file is a thin helper for
# developers who want to build from a pinned source tarball.
#
# Usage:
#   ./install.sh /path/to/privacytk-source
#
# It expects a pre-built binary at
#   <source>/target/release/privacytk
# and copies it into ~/.local/bin (or $RTK_INSTALL_DIR if set).

set -e

if [ -z "$1" ]; then
    echo "usage: $0 <path-to-privacytk-source>" >&2
    echo "  Build the binary first with 'cargo build --release' inside the source tree." >&2
    exit 2
fi

SRC="$1"
BIN="$SRC/target/release/privacytk"
INSTALL_DIR="${PRIVACYTK_INSTALL_DIR:-$HOME/.local/bin}"

if [ ! -x "$BIN" ]; then
    echo "error: $BIN not found or not executable." >&2
    echo "  Build it first: (cd '$SRC' && cargo build --release)" >&2
    exit 1
fi

mkdir -p "$INSTALL_DIR"
install -m 0755 "$BIN" "$INSTALL_DIR/privacytk"

echo "Installed privacytk to $INSTALL_DIR/privacytk"
echo "Add $INSTALL_DIR to your PATH if it isn't already."
echo
echo "Recommended next steps:"
echo "  1. privacytk verify-network api.example.com   # confirms the guard loaded"
echo "  2. privacytk telemetry status                 # confirms telemetry is off"
echo "  3. privacytk audit . --human                  # try a local scan"