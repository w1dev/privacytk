#!/usr/bin/env bash
# check-fork-drift.sh — fail CI if upstream re-introduces a forbidden pattern
# or if the fork-specific files were deleted/modified in unexpected ways.
#
# Usage:
#   scripts/check-fork-drift.sh [BASE_REF]
#     BASE_REF is anything `git diff` accepts: a commit, branch, tag, or
#     remote ref like rtk-ai/rtk@master. Default: HEAD~1 (last commit).
#
# Behavior:
#   - Greps the diff for the three classes of drift we care about:
#       * ureq::  / ureq=  / "ureq" in Cargo.toml          (telemetry channel)
#       * npx ccusage                                         (npm-registry egress)
#       * TELEMETRY_URL / endpoint_url()                     (telemetry POST)
#   - Asserts the four fork-owned files still exist with the markers
#     that distinguish them from upstream.
#   - Exits 0 on success, non-zero on the first violation. Each violation
#     prints file:line and the offending content so the reviewer can act.
#
# Designed to run in CI on every PR that touches the fork's source tree.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BASE_REF="${1:-HEAD~1}"

if ! git rev-parse "$BASE_REF" >/dev/null 2>&1; then
    echo "error: base ref '$BASE_REF' not resolvable in this repo" >&2
    exit 2
fi

# Diff stat — narrow the surface so we don't churn unrelated files. The fork
# only owns a small slice of the upstream; if a diff touches files outside
# the fork's footprint the reviewer should know.
echo "→ diff base: $BASE_REF  →  HEAD"
git diff --stat "$BASE_REF"...HEAD || true
echo

violations=0

# --- 1. ureq must not return -----------------------------------------------
echo "→ checking for ureq re-introduction"
# Source-level usages (ureq::post, ureq::get, use ureq) and the Cargo.toml
# dependency. The dep declaration line and any code that touches the crate
# are both forbidden.
ureq_hits="$(git diff "$BASE_REF"...HEAD -- \
    ':!Cargo.lock' \
    | grep -nE '^[\+].*(\bureq::|\buse ureq\b|^\+ureq\s*=)' || true)"
if [[ -n "$ureq_hits" ]]; then
    echo "  ✗ ureq references introduced:"
    echo "$ureq_hits" | sed 's/^/    /'
    violations=$((violations + 1))
else
    echo "  ✓ no ureq references in diff"
fi

# Also check the *current* Cargo.toml — a rebase could have added ureq
# without it being in the latest commit's diff vs. an older base.
if grep -qE '^ureq\s*=' Cargo.toml 2>/dev/null; then
    echo "  ✗ ureq present in Cargo.toml"
    violations=$((violations + 1))
else
    echo "  ✓ Cargo.toml has no ureq dep"
fi
echo

# --- 2. npx ccusage fallback must stay gone -------------------------------
echo "→ checking for npx ccusage fallback"
npx_hits="$(git diff "$BASE_REF"...HEAD -- src/analytics/ccusage.rs \
    | grep -nE '^[\+].*npx.*ccusage|npx.*--yes.*ccusage' || true)"
if [[ -n "$npx_hits" ]]; then
    echo "  ✗ npx ccusage fallback reintroduced:"
    echo "$npx_hits" | sed 's/^/    /'
    violations=$((violations + 1))
else
    echo "  ✓ no npx ccusage fallback in src/analytics/ccusage.rs"
fi
echo

# --- 3. telemetry POST site must stay compiled-out ------------------------
echo "→ checking for telemetry POST site"
# `endpoint_url` returning Some(...) or any ureq::post() / .send_string()
# inside core/telemetry.rs is forbidden. We also forbid re-introducing the
# TELEMETRY_URL / TELEMETRY_TOKEN compile-time env hooks in Cargo.toml.
post_hits="$(git diff "$BASE_REF"...HEAD -- src/core/telemetry.rs src/core/telemetry_cmd.rs \
    | grep -nE '^[\+].*(ureq::post|send_string|TELEMETRY_URL|TELEMETRY_TOKEN|RTK_TELEMETRY_URL)' || true)"
if [[ -n "$post_hits" ]]; then
    echo "  ✗ telemetry POST site reintroduced:"
    echo "$post_hits" | sed 's/^/    /'
    violations=$((violations + 1))
else
    echo "  ✓ no telemetry POST site in diff"
fi
echo

# --- 4. fork-owned files must still exist --------------------------------
echo "→ checking fork-owned files"
required_files=(
    "src/core/network_guard.rs"
    "src/core/audit.rs"
    "docs/CORPORATE.md"
    "scripts/check-fork-drift.sh"
)
missing=0
for f in "${required_files[@]}"; do
    if [[ ! -f "$f" ]]; then
        echo "  ✗ missing fork file: $f"
        missing=$((missing + 1))
    fi
done
if [[ $missing -gt 0 ]]; then
    violations=$((violations + 1))
else
    echo "  ✓ all fork-owned files present"
fi
echo

# --- 5. fork-owned markers must still be present --------------------------
# These are the strings that distinguish the fork's source from a raw
# upstream copy. Removing them by accident means someone botched a
# rebase; the script should fail before review.
echo "→ checking fork markers"
declare -A markers=(
    ["src/core/constants.rs"]='pub const RTK_DATA_DIR: &str = "privacytk"'
    ["src/main.rs"]='name = "privacytk"'
    ["Cargo.toml"]='name = "privacytk"'
    ["src/core/network_guard.rs"]='default_deny'
    ["src/core/audit.rs"]='Audit scanner'
)
marker_missing=0
for file in "${!markers[@]}"; do
    needle="${markers[$file]}"
    if ! grep -qF "$needle" "$file" 2>/dev/null; then
        echo "  ✗ marker missing in $file: $needle"
        marker_missing=$((marker_missing + 1))
    fi
done
if [[ $marker_missing -gt 0 ]]; then
    violations=$((violations + 1))
else
    echo "  ✓ all fork markers present"
fi
echo

# --- Summary --------------------------------------------------------------
if [[ $violations -eq 0 ]]; then
    echo "drift check: OK (no re-introductions or missing files)"
    exit 0
else
    echo "drift check: FAILED ($violations class(es) of violation)" >&2
    exit 1
fi