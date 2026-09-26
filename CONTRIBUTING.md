# Contributing to privacytk

This fork is a corporate layer over [`rtk-ai/rtk`](https://github.com/rtk-ai/rtk).
Most contribution mechanics — filter design, commit format, the
filter pass itself — live upstream. Read the upstream's
[`CONTRIBUTING.md`](https://github.com/rtk-ai/rtk/blob/master/CONTRIBUTING.md)
for the design philosophy, scope rules, and the step-by-step filter
implementation checklist.

This file documents only what is **specific to the fork**.

## What this fork maintains vs. inherits

| Concern | Owner | Where |
|---|---|---|
| Filter pass (git, cargo, go, npm, docker, kubectl, …) | Upstream | `src/cmds/*/` |
| Hook scripts installed by `init -g` | Upstream (file paths must stay) | `hooks/*`, `.claude/hooks/*` |
| CLI surface and command names | Upstream | `src/main.rs` |
| **Corporate guarantees** (this fork) | Fork | `src/core/network_guard.rs`, `audit.rs`, `cfd.rs`, `ephemeral.rs`, `dedup.rs`, `network_audit.rs`, `drift_check.rs` |
| Drift detection (this fork) | Fork | `scripts/check-fork-drift.sh`, `core::drift_check` |
| SessionStart hook injection (this fork) | Fork | `core::hooks.init.insert_corporate_hooks` |
| CHANGELOG.md | Fork (own entries; upstream changelog not tracked) | `CHANGELOG.md` |

If you touch anything outside `src/core/`, `scripts/`, `docs/CORPORATE.md`,
`README.md`, `CHANGELOG.md`, or the `Hooks` wiring in `src/main.rs`, you
are likely working on the upstream, not the fork. PR your change
upstream and let it flow in via rebase.

## Adding a new corporate module

The fork has six precedent modules under `src/core/`:

| Module | Purpose | Public surface |
|---|---|---|
| `network_guard` | Egress allowlist | `network_guard::allow(url) -> Verdict` |
| `audit` | Secrets/PII scanner | `audit::Scanner::scan_text / scan_tree` |
| `cfd` | CFD-aware ADR lookup | `cfd::search / find_by_id / session_manifest` |
| `ephemeral` | Zero-persistence mode | `ephemeral::is_active / sqlite_uri_override` |
| `dedup` | Output dedup cache | `dedup::lookup / record` |
| `network_audit` | Subprocess sandbox | `network_audit::run / detect_sandbox` |

A new module should:

1. Live under `src/core/`.
2. Have its own `is_active` / opt-in flag (default off), so the fork's
   "opt-in by default, never auto-install surprising behavior" policy
   holds.
3. Have a unit test module with at least 4 tests covering the happy
   path, the failure path, the opt-in flag, and the disabled-by-default
   state.
4. Not make outbound network calls unless it goes through
   `network_guard::allow(url)` first.
5. Not write to disk outside the ephemeral temp dir or the standard
   `~/.local/share/privacytk/` paths enumerated by `ephemeral::enumerate_state_paths`.

When you add a module:

- Register it in `src/core/mod.rs`.
- If it's a CLI subcommand, register it in `RTK_META_COMMANDS` in
  `src/core/constants.rs` and dispatch it in `main.rs`.
- Add at least one drift test to `src/core/drift_check.rs` if the
  module introduces a new invariant.
- Update `README.md` "New in the fork" section and `docs/CORPORATE.md`
  if it changes operator-visible behaviour.

## Commit messages

This fork follows Conventional Commits with the `fork:` prefix on the
subject line so a rebase from upstream is unambiguous in `git log`:

```
<type>(<scope>): <short description>
```

Use the `fork:` prefix only when the commit is exclusive to this fork.
Pure-rebase commits (filter changes, upstream-only fixes) keep the
upstream's original subject line. This way a developer scanning
`git log --oneline` can tell at a glance which commits are fork-only.

Examples:

```
fork: install SessionStart + pre-commit audit hooks via init -g
fork: drop pre-commit-gate hook (PR + human review is the gate)
fork: rebrand: replace upstream CHANGELOG with fork-only changelog
refactor: tighten `is_trae_hook_command` matcher
```

The fork's `CHANGELOG.md` is hand-written (not auto-generated) because
the upstream changelog no longer applies.

## Pre-commit gate

All five must pass before any PR:

```bash
cargo fmt --all --check
cargo clippy --all-targets
cargo test --bin privacytk --release
cargo test --bin privacytk --release core::drift_check
bash scripts/check-fork-drift.sh HEAD~1
```

The four-smoke battery from the README is mandatory before a release:

```bash
./target/release/privacytk telemetry status | grep 'this build'
./target/release/privacytk verify-network api.example.com | grep 'deny'
./target/release/privacytk audit /tmp/_smoke --human
lsof -p $(${BIN} verify-network api.example.com & echo $!) | grep -c IPv   # 0 sockets
```

## What is out of scope for this fork

- Re-implementing the filter pass in Go or another language.
  Discussed and rejected; see `CHANGELOG.md` for the conversation
  trail. The filter pass stays in Rust, the fork provides the
  corporate layer.
- Re-introducing the pre-commit audit gate by default. The fork
  assumes PR + human review. `init -g` no longer installs it.
- Adding new HTTP clients (`reqwest`, `surf`, `isahc`). All egress
  must go through `core::network_guard`.
- Adding upstream-style telemetry. The drift test enforces this.
- Re-introducing `homepage` / `repository` URLs in `Cargo.toml`
  pointing at `rtk-ai/rtk`. Drift-enforced.

## Contact

- **Upstream contributions**: see `rtk-ai/rtk`'s `CONTRIBUTING.md`.
- **Corporate fork contributions**: the operator's internal channel.
