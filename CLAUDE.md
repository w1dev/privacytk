# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working
in this repository. **This is `privacytk`, a corporate fork of
[`rtk-ai/rtk`](https://github.com/rtk-ai/rtk) v0.49.0.** The fork
keeps the upstream's compression filters and adds the corporate
guarantees documented in the README and `docs/CORPORATE.md`. This
CLAUDE.md is the fork's — it does not redirect to the upstream.

## What this repo is

`privacytk` is a CLI proxy that compresses command output before the
LLM (or the developer) reads it. It is **drop-in compatible with the
upstream `rtk` CLI surface** (same subcommands, same flags, same hooks),
but with three corporate guarantees layered on top:

1. **No telemetry.** The upstream `telemetry::maybe_ping` is removed.
   ``pkt` telemetry status` reports the build does not transmit.
2. **Default-deny network.** Every outbound URL must pass an explicit
   allowlist (`~/.config/pkt/network.toml`).
3. **Local secrets/PII scanner.** ``pkt` audit <path>` walks a tree
   and reports hits with redacted previews.

The README has the full threat model. The "How to test" section has
the four-smoke battery you must run before any release.

## Provenance

Forked from `rtk-ai/rtk` v0.49.0. The fork tracks upstream for filter
additions (new commands, new ecosystems) but rebases selectively — the
fork's `Cargo.toml` rewrites the package name to `privacytk`, removes
`ureq`, removes the `npx ccusage` fallback, and replaces the upstream
`init` with a corporate-friendly variant.

The drift check (`scripts/check-fork-drift.sh` + `cargo test
core::drift_check`) is the source of truth for "did a merge reintroduce
something the fork removed?" — run it on every PR.

## What you should NEVER do in this fork

These are the drift invariants the test suite enforces. If you are
about to do any of these, **stop and confirm with the operator**:

- Add `ureq` back to `Cargo.toml`. The drift test fails on this.
- Re-introduce `npx --yes ccusage`. Drift test fails.
- Add a `ureq::post` or `send_string` call in `src/core/telemetry.rs`
  or `src/core/telemetry_cmd.rs`. Drift test fails.
- Revert `RTK_DATA_DIR` from `"privacytk"` back to `"rtk"`. Drift test
  fails.
- Re-add `homepage` or `repository` URLs to `Cargo.toml` pointing at
  `rtk-ai/rtk`. Drift test fails.
- Re-add the pre-commit audit gate in ``pkt` init -g`. The drift
  test asserts the constant `PRE_COMMIT_GATE_HOOK_COMMAND` is absent;
  if you re-enable, also flip that assertion.

If a rebase from upstream demands any of the above, document the
exception in `CHANGELOG.md` and add a corresponding drift test
exemption.

## Development commands

Prefer the fork's own compression when iterating:

```bash
# Build (debug + release)
cargo build
cargo build --release

# Test
cargo test --bin privacytk --release
cargo test --bin privacytk --release core::drift_check   # corporate invariants
cargo test --bin privacytk --release core::audit         # scanner
cargo test --bin privacytk --release core::cfd           # CFD integration
cargo test --bin privacytk --release core::dedup         # dedup cache
cargo test --bin privacytk --release core::ephemeral      # ephemeral mode
cargo test --bin privacytk --release core::network_guard # allowlist

# Smoke battery (manual, must pass before release)
./target/release/`pkt` telemetry status | grep 'this build'
./target/release/`pkt` verify-network api.example.com | grep 'deny'
./target/release/`pkt` audit /tmp/_smoke --human
# zero outbound sockets during verify-network

# Drift
bash scripts/check-fork-drift.sh HEAD~1
```

The upstream scripts (`scripts/test-all.sh`, `scripts/benchmark.sh`,
`scripts/rtk-economics.sh`, etc.) are inherited from `rtk-ai/rtk` and
remain unmodified. They still work because the CLI surface is
identical.

## Architecture

The architecture mirrors the upstream but with corporate layers added:

- `src/cmds/*/` — filter pass, one module per ecosystem (git, cargo,
  go, npm, docker, kubectl, …). Inherited from upstream. Compression
  logic stays here.
- `src/hooks/` — Claude Code hook scripts + the init flow that wires
  them into `~/.claude/settings.json`. **The pre-commit audit gate was
  removed**; only `PreToolUse` (Bash rewrite) and `SessionStart` (CFD
  injection) are installed by `init -g`.
- `src/core/network_guard.rs` — fork-only. Default-deny TOML allowlist
  for any URL the binary would otherwise fetch.
- `src/core/audit.rs` — fork-only. Local secrets/PII scanner with
  redacted previews.
- `src/core/cfd.rs` — fork-only. CFD-aware ADR lookup + session-start
  bundle builder.
- `src/core/dedup.rs` — fork-only. LRU cache for identical command
  outputs.
- `src/core/ephemeral.rs` — fork-only. Routes tracking DB and recall
  DB to `:memory:` SQLite when `PRIVACYTK_EPHEMERAL=1` is set.
- `src/core/network_audit.rs` — fork-only. `unshare -n` / `sandbox-exec`
  wrapper for pre-flight-checking new CLIs.
- `src/core/drift_check.rs` — fork-only. 22 source-level invariants
  that mirror the bash drift script.

For the upstream's deeper architecture write-up, see
`docs/contributing/ARCHITECTURE.md` and `docs/contributing/TECHNICAL.md`
(those are upstream docs; the fork does not maintain them).

## Coding rules for this fork

Rust patterns, error handling, and anti-patterns are defined in
`.claude/rules/rust-patterns.md` (auto-loaded into context). Key
points that differ from the upstream:

- **`anyhow::Result` everywhere, always `.context("description")?`**
- **No `unwrap()` in production code.** Drift test enforces zero
  `unsafe` blocks (the upstream `lints.rust.unsafe_code = "deny"`
  still applies).
- **`LazyLock` statics** for all regex.
- **No new HTTP client.** If a feature requires making a network call,
  it must go through `core::network_guard`. Do not add `reqwest`,
  `surf`, `isahc`, or any other HTTP client to `Cargo.toml`.
- **No new `npx` / `pip` / `npm` invocations** in analytics paths. They
  are covert egress channels.
- **New secrets patterns** go into the `PATTERNS` const in
  `src/core/audit.rs`. Add a unit test next to the new pattern.
- **New corporate modules** go under `src/core/` (audit, cfd, dedup,
  ephemeral, network_guard, network_audit, drift_check are the
  precedent).

## Working directory confirmation

**ALWAYS confirm working directory before starting any work**:

```bash
pwd  # Verify you're in the privacytk project root
git branch  # Verify correct branch
git log --oneline | head -5  # Spot-check that commits are local
```

The fork's git log should show commits like `fork: install SessionStart...`,
`fork: `pkt` cfd + cfd-context...`, `fork: drop pre-commit-gate...`.
If you see commits referencing `rtk-ai/rtk` URLs without a "fork:" prefix,
you are on the wrong checkout.

## Avoiding rabbit holes

- **Do not research upstream `rtk-ai/rtk` issues / PRs.** The fork
  rebases selectively. Open PRs upstream may never land here.
- **Do not "fix" the upstream scripts in `scripts/`.** They work
  because the CLI surface is identical. If they break, the cause is
  in the binary, not in the script.
- **Do not refactor `src/cmds/*/`.** Those are the upstream filter
  pass. The fork adds corporate layers above; the filter logic is
  intentionally untouched so rebase diffs stay small.
- **Do not "fix" the upstream's CLAUDE.md, CONTRIBUTING.md, or
  AWARENESS_CONFIG.md unless the fork's changes require it.** The
  fork's CLAUDE.md is this file; the upstream's is at `rtk-ai/rtk`.

## Plan execution protocol

When the operator provides a numbered plan, follow it sequentially.
Commit after each logical step. Run the drift check + the four-smoke
battery before pushing or merging. The drift check is the merge
gate; the smoke battery is the release gate.

If a step is blocked, report it and ask before proceeding — do not
silently skip or reorder.
