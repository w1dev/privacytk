## Summary
<!-- What does this PR do? Keep it short (1-3 bullet points). -->

-

## Test plan
<!-- How did you verify this works? -->

- [ ] `cargo fmt --all && cargo clippy --all-targets && cargo test`
- [ ] `cargo test --bin privacytk --release core::drift_check`
- [ ] `bash scripts/check-fork-drift.sh HEAD~1`
- [ ] Four-smoke battery from the README (telemetry status, verify-network, audit, zero sockets)
- [ ] Manual testing: `privacytk <command>` output inspected (where applicable)

## Corporate invariants

<!-- If your PR touches any of these, explain why. -->

- [ ] No new HTTP client (`ureq`, `reqwest`, `surf`, `isahc` …) added to `Cargo.toml`.
- [ ] No `npx` / `pip` / `npm` invocations added in `src/`.
- [ ] No outbound URLs added without going through `core::network_guard`.
- [ ] No re-introduction of upstream telemetry (`TELEMETRY_URL`, `RTK_*` env vars).
- [ ] No `homepage` / `repository` URLs pointing at `rtk-ai/rtk`.

## Provenance

<!-- privacytk forks rtk-ai/rtk v0.49.0. Filter changes go upstream; corporate
     changes (audit, cfd, ephemeral, dedup, network_guard, network_audit,
     drift_check, hooks installer) stay here. -->

Is this a rebase from upstream, a fork-only addition, or both?

> See [CLAUDE.md](../blob/master/CLAUDE.md) and
> [docs/CORPORATE.md](../blob/master/docs/CORPORATE.md) for the
> fork's working agreements.
