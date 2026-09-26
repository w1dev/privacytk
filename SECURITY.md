# Security policy

privacytk is a corporate fork of [`rtk-ai/rtk`](https://github.com/rtk-ai/rtk)
v0.49.0. The fork does not run its own security disclosure process —
it inherits the upstream's policy at `rtk-ai/rtk/blob/master/SECURITY.md`
and re-uses the same vulnerability reporting channels.

## Reporting a vulnerability

Report vulnerabilities upstream to `rtk-ai/rtk` via:

- **Private security advisory** on the `rtk-ai/rtk` GitHub repo.
- **Email**: `security@rtk-ai.app`.

Once a fix is published upstream, this fork picks it up on the next
rebase. If your operator needs an out-of-band fix for a corporate-only
surface (the `audit`, `cfd`, `ephemeral`, `network_guard`,
`network_audit`, `dedup` modules — all under `src/core/`), open a
private issue on the internal fork.

**Please do NOT** open public GitHub issues for security vulnerabilities.

## Corporate-specific invariants

The fork adds three corporate guarantees on top of the upstream
CLI. They are enforced by tests, not by `unsafe` code paths:

| Invariant | Test | Enforcement |
|---|---|---|
| `ureq` is not in `Cargo.toml` | `cargo test core::drift_check::cargo_toml_does_not_declare_ureq` | drift check + cargo test |
| Telemetry POST site is compiled out | `core::drift_check::telemetry_module_has_no_post_site` | drift check + cargo test |
| `npx ccusage` fallback is removed | `core::drift_check::ccusage_drops_npx_fallback` | drift check + cargo test |

These run on every PR (`.github/workflows/fork-ci.yml`). If any
violates, the merge is blocked.

## Sensitive files

The fork adds new sensitive files beyond the upstream's list:

- `src/core/network_guard.rs` — the egress allowlist. A bug here could
  allow the binary to phone home.
- `src/core/audit.rs` — the secrets scanner. A bug could mask real
  findings or generate false negatives.
- `src/core/cfd.rs` — parses untrusted documents (`CLAUDE.md`,
  `_index.md`) and feeds them into the agent context. A parsing bug
  could inject content into the LLM.
- `src/hooks/init/mod.rs::insert_corporate_hooks` — writes to the
  agent's settings.json. A bug could install the wrong hooks.
- `install.sh` — runs `rustup` and `cargo` on the operator's machine.
  A bug could install the wrong toolchain or write to the wrong path.

Modifying these requires an extra reviewer and a re-run of the drift
check + the four-smoke battery from the README.

## What the fork does NOT do (security-relevant)

- The fork does not transmit telemetry. Verified by
  `cargo test core::drift_check::telemetry_module_has_no_post_site`.
- The fork does not embed upstream release artefacts. The CI is
  source-built, signed, and the operator verifies SHA-256 + cosign.
- The fork does not auto-update. There is no self-update mechanism.
  Updates come via your package repo (apt/dnf/OCI) on the operator's
  schedule.

If any of these change, the corresponding drift test will fail.

## Contact

- **Upstream vulnerabilities**: `security@rtk-ai.app` (see upstream
  `SECURITY.md`).
- **Corporate fork issues**: the operator's internal channel.
- **Public discussion**: `https://github.com/rtk-ai/rtk/discussions`.
