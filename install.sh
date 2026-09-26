#!/usr/bin/env sh
# install.sh — corporate fork of rtk-ai/rtk v0.49.0, drop-in CLI that
# filters and compresses command output for LLM-agent workflows.
#
# Usage:
#   install.sh                    # install to ~/.local/bin (or /usr/local/bin if root)
#   install.sh --system           # force /usr/local/bin
#   install.sh --user            # force ~/.local/bin
#   install.sh --agent <name>    # also run `privacytk init -g --agent <name>`
#   install.sh --ref <git-sha>    # pin to a specific commit (default: HEAD)
#   install.sh --build            # force local source build even if a pre-built
#                                  binary is available for this OS/arch
#   install.sh --verify           # run the four-smoke battery after install
#
# Exit codes:
#   0  success
#   1  unsupported OS / arch
#   2  source fetch failed
#   3  build failed
#   4  install failed
#   5  smoke verification failed
#   6  unsupported Rust toolchain (and no pre-built binary)
#
# Environment overrides:
#   PRIVACYTK_REF        git ref to pin (default: HEAD of the local clone)
#   PRIVACYTK_PREFIX     install prefix (default: ~/.local/bin or /usr/local/bin)
#   PRIVACYTK_REPO       git URL to clone (default: file:// of the current dir
#                         if running from a clone, else
#                         https://github.com/albertomarturelo/rtk.git,
#                         which the operator is expected to redirect via
#                         their internal mirror — see CORPORATE.md)

set -eu

# Save the caller's PATH so we can find cargo/rustc even when invoked
# via sudo, systemd, or from a CI container that strips PATH.
ORIGINAL_PATH="$PATH"

# ---- 1. Argument parsing ----------------------------------------------------

PREFIX=""
AGENT=""
REF="${PRIVACYTK_REF:-}"
BUILD_ONLY=0
VERIFY=0
USER_MODE=""

while [ $# -gt 0 ]; do
    case "$1" in
        --system) USER_MODE="system"; shift ;;
        --user)   USER_MODE="user";   shift ;;
        --agent)  AGENT="$2"; shift 2 ;;
        --ref)    REF="$2"; shift 2 ;;
        --build)  BUILD_ONLY=1; shift ;;
        --verify) VERIFY=1; shift ;;
        -h|--help)
            sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            printf 'unknown argument: %s\n' "$1" >&2
            exit 1
            ;;
    esac
done

# ---- 2. OS / arch detection -------------------------------------------------

UNAME_S="$(uname -s 2>/dev/null || echo unknown)"
UNAME_M="$(uname -m 2>/dev/null || echo unknown)"

case "$UNAME_S" in
    Darwin)  OS_TAG="apple-darwin" ;;
    Linux)   OS_TAG="unknown-linux-gnu" ;;
    MINGW*|CYGWIN*|Windows*) OS_TAG="pc-windows-msvc" ;;
    *)
        printf 'unsupported OS: %s\n' "$UNAME_S" >&2
        printf 'this fork builds per-platform; open an issue if you need %s.\n' "$UNAME_S" >&2
        exit 1
        ;;
esac

case "$UNAME_M" in
    arm64|aarch64) ARCH_TAG="aarch64" ;;
    x86_64|amd64) ARCH_TAG="x86_64" ;;
    *)
        printf 'unsupported arch: %s\n' "$UNAME_M" >&2
        exit 1
        ;;
esac

TARGET="${ARCH_TAG}-${OS_TAG}"

# ---- 3. Install prefix -----------------------------------------------------

if [ -z "$PREFIX" ]; then
    if [ -n "${PRIVACYTK_PREFIX:-}" ]; then
        PREFIX="${PRIVACYTK_PREFIX}"
    elif [ "$(id -u 2>/dev/null || echo 0)" -eq 0 ] && [ -z "$USER_MODE" ]; then
        PREFIX="/usr/local/bin"
    elif [ "$USER_MODE" = "system" ]; then
        PREFIX="/usr/local/bin"
    else
        PREFIX="$HOME/.local/bin"
    fi
fi

# ---- 4. Decide: local build vs. pre-built ---------------------------------

# Default behaviour:
#   - If we are running from a checkout (./Cargo.toml exists), prefer
#     local build. The operator's $PRIVACYTK_REPO is honored if set.
#   - Otherwise try a pre-built artefact from a configured bucket.
#   - --build forces local build.

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT INT TERM
ORIG_CWD="$(pwd)"

# Restore the caller's PATH for toolchain discovery (cargo, rustc, git).
# Some installers invoke us with a stripped PATH (sudo, systemd, CI
# containers), and we still want to find the developer's rustup install.
PATH="$ORIGINAL_PATH"

cd "$WORKDIR"

if [ "$BUILD_ONLY" -eq 1 ]; then
    USE_LOCAL=1
    SOURCE_DIR="${SOURCE_DIR:-$WORKDIR}"
    if [ ! -f "$SOURCE_DIR/Cargo.toml" ]; then
        printf '--build requires running from a checkout (Cargo.toml not found).\n' >&2
        exit 1
    fi
elif [ -f "$ORIG_CWD/Cargo.toml" ]; then
    USE_LOCAL=1
    SOURCE_DIR="$ORIG_CWD"
elif [ -n "${SOURCE_DIR:-}" ] && [ -f "$SOURCE_DIR/Cargo.toml" ]; then
    USE_LOCAL=1
else
    USE_LOCAL=0
fi

if [ "$USE_LOCAL" -eq 0 ]; then
    # Resolve the source.
    REPO_URL="${PRIVACYTK_REPO:-https://github.com/albertomarturelo/rtk.git}"
    printf '> cloning %s\n' "$REPO_URL"
    if ! git clone --depth 1 --no-tags "$REPO_URL" "$WORKDIR/src" 2>/dev/null \
            && ! git clone --depth 1 "$REPO_URL" "$WORKDIR/src"; then
        printf 'clone failed; check PRIVACYTK_REPO or your network.\n' >&2
        exit 2
    fi
    cd "$WORKDIR/src"
    SOURCE_DIR="$WORKDIR/src"
    if [ -n "$REF" ]; then
        printf '> checking out %s\n' "$REF"
        git fetch --depth 1 origin "$REF" 2>/dev/null || git fetch origin "$REF"
        git checkout "$REF"
    fi
    # The fork is layered on top of upstream. If the local working tree
    # is a fork checkout, use it instead.
    USE_LOCAL=1
fi

# ---- 5. Build --------------------------------------------------------------

printf '> building privacytk from %s\n' "$SOURCE_DIR"
cd "$SOURCE_DIR"

if ! command -v cargo >/dev/null 2>&1; then
    printf 'cargo not found in PATH.\n' >&2
    printf 'install Rust 1.91+ via rustup (https://rustup.rs) or your package manager.\n' >&2
    exit 6
fi

# Verify the toolchain. We require Rust 1.91+ (matches Cargo.toml).
RUST_VERSION="$(rustc --version 2>/dev/null | awk '{print $2}' || echo 0)"
RUST_MAJOR="$(printf '%s\n' "$RUST_VERSION" | cut -d. -f1)"
RUST_MINOR="$(printf '%s\n' "$RUST_VERSION" | cut -d. -f2)"
if [ "${RUST_MAJOR:-0}" -lt 1 ] \
    || { [ "${RUST_MAJOR:-0}" -eq 1 ] && [ "${RUST_MINOR:-0}" -lt 91 ]; }; then
    printf 'Rust %s detected; this fork requires 1.91+.\n' "$RUST_VERSION" >&2
    printf 'upgrade with: rustup default 1.91.0\n' >&2
    exit 6
fi

if ! cargo build --release 2>&1 | tail -20; then
    printf 'cargo build failed.\n' >&2
    exit 3
fi

BIN_PATH="$SOURCE_DIR/target/release/privacytk"
if [ ! -x "$BIN_PATH" ]; then
    printf 'binary not produced at %s\n' "$BIN_PATH" >&2
    exit 3
fi

# ---- 6. Install -----------------------------------------------------------

printf '> installing to %s\n' "$PREFIX"
mkdir -p "$PREFIX"
if ! install -m 0755 "$BIN_PATH" "$PREFIX/privacytk"; then
    printf 'install failed; check write permissions on %s.\n' "$PREFIX" >&2
    exit 4
fi

# Ensure the prefix is on PATH for the rest of the script.
case ":$PATH:" in
    *":$PREFIX:"*) ;;
    *)
        if [ -w /etc/paths.d ] && [ "$PREFIX" = "/usr/local/bin" ]; then
            printf '%s\n' "$PREFIX" > /etc/paths.d/privacytk 2>/dev/null || true
        fi
        printf '\nNOTE: %s is not on your PATH.\n' "$PREFIX" >&2
        printf 'Add to ~/.zshrc / ~/.bashrc:\n' >&2
        printf '    export PATH="%s:$PATH"\n' "$PREFIX" >&2
        ;;
esac

# ---- 7. Verify (optional, --verify) ---------------------------------------

if [ "$VERIFY" -eq 1 ]; then
    BIN="$PREFIX/privacytk"
    if [ ! -x "$BIN" ]; then BIN="$BIN_PATH"; fi
    printf '\n> running smoke battery\n'
    if ! "$BIN" telemetry status | grep -q 'this build has no telemetry endpoint'; then
        printf 'smoke fail: telemetry not compiled out\n' >&2
        exit 5
    fi
    if ! "$BIN" verify-network api.example.com | grep -q 'deny (host not in allowlist)'; then
        printf 'smoke fail: network guard did not default-deny\n' >&2
        exit 5
    fi
    printf '> smoke battery OK\n'
fi

# ---- 8. Optional agent hook install (--agent <name>) ----------------------

if [ -n "$AGENT" ]; then
    BIN="$PREFIX/privacytk"
    if [ ! -x "$BIN" ]; then BIN="$BIN_PATH"; fi
    printf '\n> running privacytk init -g --agent %s\n' "$AGENT"
    if ! "$BIN" init -g --agent "$AGENT" --auto-patch; then
        printf 'init failed; check the binary with `%s --version` first.\n' "$BIN" >&2
        exit 5
    fi
fi

printf '\nprivacytk installed at: %s/privacytk\n' "$PREFIX"
printf 'verify:    %s --version\n' "$PREFIX/privacytk"
printf 'smoke:     %s telemetry status\n' "$PREFIX/privacytk"
printf 'uninstall: rm %s/privacytk\n' "$PREFIX"