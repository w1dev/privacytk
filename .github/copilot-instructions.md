# Copilot instructions for privacytk

privacytk is a corporate fork of [`rtk-ai/rtk`](https://github.com/rtk-ai/rtk)
v0.49.0. It is a CLI proxy that filters and compresses command outputs
before they reach an LLM context. This file is the fork's; the upstream
instructs Copilot to type `rtk` everywhere — that instruction does not
apply here. Use `privacytk` instead, or nothing at all (the hook
re-writes commands automatically).

## Using privacytk in this session

When you would normally run a CLI tool, run it through `privacytk`:

```bash
# Equivalent to `git status`, with token compression applied:
privacytk git status

# Or, if the SessionStart hook is installed, just run `git status`
# and the hook rewrites it for you.
git status
```

The binary lives at `target/release/privacytk` after `cargo build
--release`, or in `~/.local/bin/privacytk` after `./install.sh`.

## Build, test, smoke

```bash
cargo build --release
cargo test --bin privacytk --release
cargo test --bin privacytk --release core::drift_check
bash scripts/check-fork-drift.sh HEAD~1
```

The four-smoke battery in the README is the release gate.

## Corporate invariants

privacytk's corporate layer adds three guarantees over the upstream
CLI. **Do not break them.** Drift tests enforce each one.

1. **No telemetry.** `ureq` is not in `Cargo.toml`. The drift test
   asserts it stays out.
2. **Default-deny network.** Any new outbound URL must go through
   `core::network_guard::allow()`. Adding `reqwest`/`surf`/`isahc` is a
   drift violation.
3. **Local secrets/PII scanner.** `privacytk audit` is the scanner; new
   patterns go into `PATTERNS` in `src/core/audit.rs` with a unit test.

## Working directory

Before starting work, verify you are in the **privacytk** project root.
Check `git log --oneline | head -3` — fork commits start with `fork:`,
upstream merges do not. If you see `rtk-ai/rtk` URLs without the
`fork:` prefix, you are on the wrong checkout.

## What is out of scope

- Adding new HTTP clients.
- Re-introducing upstream telemetry or the `npx ccusage` fallback.
- Refactoring the filter pass (`src/cmds/*/`). The filter pass is the
  upstream's; it is intentionally untouched so rebase diffs stay small.
- Refactoring the upstream hook scripts in `hooks/`. Their filenames
  must stay (`rtk-rewrite.sh`, etc.) for the integration with Claude
  Code to keep working.

See [CLAUDE.md](../../blob/master/CLAUDE.md) for the full working
agreement.
