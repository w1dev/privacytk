# Changelog

All notable changes to **privacytk** are documented in this file.

privacytk is a corporate fork of [`rtk-ai/rtk`](https://github.com/rtk-ai/rtk)
pinned at [v0.49.0](https://github.com/rtk-ai/rtk/releases/tag/v0.49.0).
For the upstream changelog see the `rtk` releases page. This file
only tracks what the fork itself changed on top of the upstream base.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Corporate guarantees (added on top of upstream v0.49.0)

- **Removed**: `ureq` dependency. The upstream `telemetry::maybe_ping`
  that POSTed a usage ping is gone. `telemetry status` reports
  `(not applicable; this build has no telemetry endpoint)`.
- **Removed**: the `npx --yes ccusage` fallback in `src/analytics/ccusage.rs`.
  The fallback hit the npm registry silently on every run; the corporate
  fork refuses to invoke `npx` for analytics.
- **Added**: `core::network_guard`. Default-deny TOML allowlist at
  `~/.config/privacytk/network.toml`. The guard refuses to load any
  `default_deny = false` policy. `privacytk verify-network <host>` is
  the operator's microscope for the allowlist.
- **Added**: `core::audit`. Local-only secrets and PII scanner.
  Pattern table covers AWS keys (`AKIA…`), GitHub tokens (`ghp_/gho_/ghs_/ghu_`),
  Stripe (`sk_live_/sk_test_/rk_live_`), Google API keys (`AIza…`),
  PEM private keys, JWTs, IBANs, PANs, emails, IPv4 addresses, and URLs
  with embedded credentials. Hits are reported with redacted previews
  (`ghp_…XX (40 chars)`); raw secrets never appear in output.
- **Added**: `--ephemeral` / `PRIVACYTK_EPHEMERAL=1`. Routes tracking DB,
  recall DB, and tee buffers to `:memory:` SQLite or a temp dir removed at
  process exit. Default off; opt-in per developer or per session.
- **Added**: `core::dedup` and `--dedup` / `PRIVACYTK_DEDUP=1`. LRU cache
  of identical command outputs (5-minute TTL, capped at 256 entries).
  When the hash matches a recent invocation, the LLM receives a one-line
  hit instead of the duplicated bytes. Cache lives under
  `/tmp/privacytk-dedup-<user>/`; only hashes are stored, never output.
- **Added**: `core::ephemeral` (settings inventory). `privacytk where-state`
  lists every path the binary can touch on disk (SQLite DBs, TOML
  config, tee buffers, hook-audit log, telemetry markers) with size and
  existence. `privacytk where-state --purge` removes the ephemeral temp dir.
- **Added**: `core::network_audit`. `privacytk network-audit <cmd>` runs
  the supplied command under `unshare --net` (Linux) or `sandbox-exec`
  (macOS) and reports the blocked TCP attempts. Pre-flight check for
  any new wrapper CLI the operator wants to adopt.
- **Added**: `core::cfd`. `privacytk cfd <query>` parses the
  `docs/decisions/_index.md` (both table and bullet forms), searches by
  title/id/status, and returns individual ADRs by id (with `--summary`
  for a one-liner). `privacytk cfd-context --top-adrs N` builds the
  session-start bundle (CLAUDE.md + index + In Progress ADRs) per the
  CFD methodology.

### Hook behaviour

- **Renamed**: every Claude/Cursor/Codex/etc. hook command now points at
  `privacytk hook <agent>` instead of `rtk hook <agent>`. `is_rtk_binary()`
  accepts both names so operators with both installs in `PATH` keep
  working.
- **Removed**: `privacytk init -g` no longer installs a pre-commit audit
  gate. The corporate fork assumes a PR-based flow with human review
  and GHE push protection; the audit stays available as a manual
  command (`privacytk audit <path>`) and as a CI step.
- **Added**: `SessionStart` hook. `privacytk init -g` installs
  `privacytk hook session-start` which emits the CFD manifest on stdout.
  Claude Code prepends it to the session's opening context automatically.

### Drift detection

- **Added**: `scripts/check-fork-drift.sh`. POSIX-portable bash script
  that greps the diff against upstream for forbidden patterns
  (`ureq::`, `npx ccusage`, telemetry POST site), asserts that the
  four fork-owned files still exist, and asserts the fork's
  distinguishing markers are present.
- **Added**: `core::drift_check`. 21 unit tests that mirror the bash
  script: assert `ureq` is absent from `Cargo.toml`, telemetry module
  has no POST site, ccusage has no `npx` invocation, hooks are wired
  via `insert_corporate_hooks`, etc. Runs in CI without external dependencies.
- **Added**: `.github/workflows/fork-ci.yml`. Lean CI that runs the
  drift script, drift tests, release build, smoke battery, and an
  audit synthetic-secrets test. No third-party API keys, no Docker.

### Build and packaging

- **Changed**: package `name` from `rtk` to `privacytk`. Binary name
  follows. `RTK_DATA_DIR` (the user-local SQLite directory) renamed to
  `privacytk` so a fork install never collides with an upstream install.
- **Changed**: `homepage` and `repository` removed from `Cargo.toml`.
  The fork is internal; the binary must not redirect to upstream URLs.
- **Changed**: `Cargo.lock` regenerated; `ureq`, `ureq::post`,` etc. not
  present in the dependency graph.

### Documentation

- **Added**: `README.md` rewritten as a corporate-focused document with
  a provenance block at the top.
- **Added**: `docs/CORPORATE.md`. Threat model, image packaging recipe,
  allowlist maintenance, drift check automation, audit log integration,
  subprocess network exposure table, automatic hooks reference,
  emergency revocation procedure.
- **Added**: `install.sh`. The upstream curl-from-GitHub installer is
  gone; the corporate installer copies a pre-built binary from a pinned
  source tarball.
- **Removed**: upstream `INSTALL.md` and `Formula/rtk.rb` (Homebrew
  formula pointing at the upstream tap).

### Tests

- **Added**: 19 cargo unit tests for `core::audit` (AWS, GitHub,
  PEM, JWT, IBAN, PAN, email, IPv4, URL-with-creds, redact-preview
  edge cases, including the four-prefix drift test).
- **Added**: 12 cargo unit tests for `core::cfd` (table-form parsing,
  bullet-form parsing, search AND-semantics, filter-by-status,
  find-by-id with or without `ADR-` prefix, render, summarize,
  trim_status, locate_index, slugify).
- **Added**: 6 cargo unit tests for `core::dedup` (key stability,
  fresh/hit/record, TTL expiry, env var recognition).
- **Added**: 11 cargo unit tests for `core::network_audit`
  (sandbox detection, empty argv handling, tail truncation, render).
- **Added**: 21 cargo unit tests for `core::drift_check` (the
  drift invariants enumerated above).
- **Status**: integration tests in `tests/` are upstream tests that
  reference `CARGO_BIN_EXE_rtk`. They do not compile in the fork.
  The cargo unit suite is the test of record for privacytk; the
  integration tests are tracked as a separate round if your operator
  needs them.

## How to read this changelog

Each entry follows `<Type>: <summary>` where Type is one of
**Added**, **Changed**, **Removed**, **Fixed**, **Deprecated**.
Types follow the Keep a Changelog convention.

For changes inherited from upstream `rtk` v0.49.0, see the
[upstream CHANGELOG](https://github.com/rtk-ai/rtk/blob/master/CHANGELOG.md).
This file only documents what privacytk itself did.