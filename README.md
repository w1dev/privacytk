# privacytk

> Forked from [`rtk-ai/rtk`](https://github.com/rtk-ai/rtk) at
> [v0.49.0](https://github.com/rtk-ai/rtk/releases/tag/v0.49.0).
> privacytk keeps rtk's compression filters and adds the corporate
> guarantees below. Apache-2.0, inherited from upstream. The
> `forked from` clause is the only place this README names rtk; the
> binary, subcommands, hooks, and configuration all live under the
> `privacytk` namespace.

A corporate fork of [rtk](https://github.com/rtk-ai/rtk) tuned for environments
where the CLI cannot, by policy, exfiltrate code, secrets, or usage data.

The fork preserves 100% of the upstream's token-compression filters for
LLM-agent workflows (`git`, `cargo`, `cargo test`, `pytest`, `go test`,
`docker ps`, `kubectl`, etc.) and adds three corporate-only guarantees:

1. **No telemetry.** The `ureq`-backed telemetry POST is gone. `ureq`
   is no longer in `Cargo.toml`. The `telemetry status` command reports
   `(not applicable; this build has no telemetry endpoint)`.
2. **Default-deny network.** Every outbound URL must pass
   `core::network_guard`, which reads an explicit allowlist from
   `~/.config/privacytk/network.toml`. The guard refuses to load any
   `default_deny = false` policy.
3. **Local secrets & PII scanner.** `privacytk audit <dir>` walks a tree and
   reports matches (AWS keys, GitHub tokens, JWTs, IBANs, PAN-shaped runs, etc.)
   with redacted previews. Never opens a socket.

## Build

Requires Rust 1.91 or newer (matches `rust-version` in `Cargo.toml`).

```bash
cargo build --release
# binary at target/release/privacytk
```

The toolchain is keg-only under Homebrew; if `cargo` is not on `PATH`, prepend
`/opt/homebrew/opt/rustup/bin`.

## Install the network allowlist

`privacytk` refuses to talk to anything until you write an allowlist. Default
location (overridable via `PRIVACYTK_NETWORK_CONFIG`):

```
~/.config/privacytk/network.toml   (macOS / Linux)
%APPDATA%\privacytk\network.toml   (Windows)
```

Minimal policy:

```toml
schema_version = 1
default_deny = true

allow = [
  # Add the hosts your LLM stack actually needs.
  # "api.internal.example.com",
]
```

A permissive policy (`default_deny = false`) is rejected at load time.

## Commands

### Existing commands

All of the upstream CLI surface works unchanged: `git`, `cargo`, `gh`, `glab`, `aws`,
`docker`, `kubectl`, `ls`, `grep`, `find`, `read`, `err`, `test`, `json`,
`deps`, `env`, `log`, `diff`, `summary`, `smart`, etc. See the upstream
README (linked at the top of this file) for the full table — only the names
`privacytk init` and `privacytk hook` require per-agent path rewrites that
still target the same hooks directory.

### New in the fork

#### `privacytk audit <path> [--human]`

Walks `<path>` (default `.`) and scans every regular file for matches against
the pattern table in `src/core/audit.rs`. Files starting with `.git/`, symlinks,
and binary files (NUL byte heuristic) are skipped.

- Default: JSON output to stdout, suitable for piping into a CI report.
- `--human`: human-readable lines `path:line:col  category  redacted-preview`.

Exit code: `0` when no hits, `1` when at least one hit. The scanner never
opens a socket — no data leaves the host.

#### `privacytk verify-network [host ...] [--reload]`

Loads the network policy and prints:

- the resolved config path,
- `default_deny` value,
- the allowlist,
- per-host verdicts for every host or URL passed as an argument.

`--reload` re-reads the config from disk before evaluating. No socket is
opened even when hosts are passed — the verdict is computed purely against the
allowlist.

#### `privacytk telemetry status|enable|disable|forget`

`enable` no longer transmits anything; it asks for an acknowledgement so an
audit log can record the operator intent. `forget` deletes the (already
empty) salt file, the marker file, and the local tracking DB.

## Threat model

| Channel | Upstream | privacytk |
|---|---|---|
| Daily telemetry POST to upstream collector | yes (opt-in) | **compiled out** |
| `npx ccusage` fallback in analytics | yes (network call) | **removed** |
| `ureq` crate in dependency graph | yes | **removed** |
| Outbound URL from any filter | unrestricted | **allowlist-only** |
| Local secrets/PII scanning | none | **built-in** |

What privacytk does *not* address:

- The user can still pipe a secret into a `curl`/`wget` filter themselves.
  The fork adds no shim around those commands; treat them as user-invoked
  network actions.
- The hook rewriter (`privacytk init`) still installs a `~/.claude/hooks/*`
  script that calls `privacytk` directly. If you replace `privacytk`'s binary
  with one from another source, the hook will execute that code. Pin your
  binary in your image.

## License

Apache-2.0, inherited from the upstream project. See `LICENSE`.