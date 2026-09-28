# Corporate deployment guide

> **Provenance.** `pkt` is a corporate fork of
> [rtk-ai/rtk v0.49.0](https://github.com/rtk-ai/rtk/releases/tag/v0.49.0).
> All token-compression filters, hook scripts, and benchmark suite come
> from upstream. The corporate modifications (telemetry removal,
> default-deny network guard, audit scanner, ephemeral mode, CFD
> integration, drift-check harness) are listed in the README under
> "Why this fork exists". This document covers what the README does
> not: image packaging, policy files, drift detection, and operational
> audits.

This document covers the operational concerns that aren't in the README:
image packaging, policy files, and what to audit when you onboard `pkt`
into a regulated environment.

## 1. Threat model recap

privacytk is a CLI proxy that runs in the shell of every developer in your
organization. It sees:

- every command they type (via the LLM-agent hook rewriter),
- every byte of every tool's output that the agent is about to read,
- the filesystem around the developer's working tree.

That makes it a high-value target: a bug or a hostile update can exfiltrate
source code, secrets, or business data. The fork's job is to make that
harder.

| Risk | Mitigation in privacytk |
|---|---|
| Silent telemetry upload | `ureq` removed from the dependency graph. `telemetry::endpoint_url()` is `None`. `telemetry::maybe_ping()` is a no-op. |
| Implicit registry lookup | `npx ccusage` fallback removed from `src/analytics/ccusage.rs`. ccusage analytics only runs when the binary is on `PATH` in the corporate image. |
| Filter author ships a network call | New outbound traffic must pass `core::network_guard::allow`, which is deny-by-default. |
| Operator hands the agent a secrets-laced tree | `privacytk audit` flags AWS keys, GitHub tokens, JWTs, IBANs, PAN-shaped runs, etc. with redacted previews. |
| Accidental permissive policy | `network_guard::load_config()` rejects `default_deny = false` at parse time. |
| Drift between dev image and CI image | Single `Cargo.toml` with `publish = false` and explicit `[lints.rust]` — builds are deterministic. |

## 2. Packaging for a corporate image

The recommended pattern is a **two-layer image**:

1. **Base image** — your hardened OS image (Ubuntu LTS, RHEL UBI, etc.).
2. **Tooling image** — derived, installs `privacytk` from a pinned source
   tarball, drops the network policy into `/etc/skeleton/.config/privacytk/`,
   and (optionally) installs the Claude Code hook.

A minimal Dockerfile:

```dockerfile
FROM your-base:tag AS base

# 1. Install Rust toolchain
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain 1.91.0 --profile minimal

# 2. Build privacytk from a pinned source
ARG PRIVACYTK_COMMIT=pin-me
RUN git clone https://internal.example.com/privacytk.git /src && \
    cd /src && git checkout "$PRIVACYTK_COMMIT" && \
    /root/.cargo/bin/cargo build --release

# 3. Lay down the deny-by-default policy (operator overrides later)
COPY network.toml /etc/skel/.config/privacytk/network.toml

# 4. Drop the binary into PATH
RUN install -m 0755 /src/target/release/privacytk /usr/local/bin/privacytk
```

The `network.toml` shipped with the image should be:

```toml
schema_version = 1
default_deny = true
allow = []
```

i.e. **empty allowlist**. Operators add hosts in their own per-user override
file, which is what `verify-network` reads first.

## 2b. Token savings: measured baselines and the dedup opt-in

privacytk compresses command output in-memory before sending it to the
LLM. The compression is local — no bytes leave the host during the
filter pass. Three measured baselines from a representative workload
(captured `/tmp/privacytk-bench/`, reproducer in `scripts/bench-corpus.sh`):

| Command | Raw bytes | privacytk bytes | Reduction |
|---|---:|---:|---:|
| `cargo test` (50 failures + 25 passed) | 13096 | 9798 | -25% |
| `git diff` (8 files changed) | 22523 | 21793 | -3% |
| `kubectl get pods -A` (~220 lines, mostly Running) | 20046 | 1153 | **-94%** |
| `pytest -v` (12 failures over 80 tests, simulated) | 5451 | ~3000 (estimated) | -45% |

The `kubectl` case is where privacytk shines (grouping, cap, dedup of
identical rows). The `git diff` case is the worst — the filter passes
the diff almost literally. To improve that case, see `dedup` below.

### Dedup: opt-in dedup of identical re-runs

Most developer workflows run the same command repeatedly while
iterating: `cargo test` → fix → `cargo test` → fix → ... The output is
usually the same on every iteration. With `--dedup` or
`PRIVACYTK_DEDUP=1`, privacytk hashes each output and stores the hash
under `/tmp/privacytk-dedup-<user>/cache.jsonl` (cap 256 entries, TTL 5
minutes). On a hit, the LLM receives a one-liner instead of the full
output:

```
git status: output identical to 18s ago (1234 bytes, sha=ab12cd34) —
  pass `--no-dedup` to see the full output
```

Measured savings on identical re-runs: **99.2%** of the duplicated bytes
do not reach the LLM. In a 5-iteration debugging session, that is 4
full outputs replaced by 4 one-liners.

Privacy properties:
- The cache stores `(cwd, argv, sha256, bytes_len, timestamp)`. It does
  **not** store the raw output. An attacker reading `/tmp` cannot
  reconstruct what the developer ran.
- The cache lives under `/tmp/`, never inside the repo or user home.
  When `--dedup` is not active (or the binary exits cleanly with
  `--dedup` never toggled on), `DedupGuard::drop()` removes the cache.
- `--dedup` is **opt-in only**. Off by default. To enable per
  developer, add `export PRIVACYTK_DEDUP=1` to the corporate shell
  profile; to enable per project, set it in `.envrc`.

Inspection commands:

```bash
privacytk dedup           # show cache path + status
privacytk dedup clear     # wipe cache
privacytk dedup set on    # toggle at runtime
privacytk dedup lookup "<cwd>::<argv>"   # check whether a key would hit
```

Recommended pattern for CI: do **not** enable `--dedup` in CI. CI runs
each command exactly once; the cache would only matter if a flaky test
masked a real regression. Keep CI on the uncompressed output so the
LLM sees fresh data every time.

## 2c. CFD integration (`privacytk cfd`)

[Context-First Development](https://github.com/albertomarturelo/context-first-development)
(CFD) is a methodology, not a search algorithm. The pain point it
addresses — *the agent reads entire files when it only needs a slice*
— is what `privacytk cfd` solves.

Two subcommands:

```bash
# Print the decisions index as a compact table.
privacytk cfd [--repo PATH]

# Search by topic (AND of terms across id, title, status).
privacytk cfd "jwt validation" [--repo PATH]

# Read a single ADR by id.
privacytk cfd --id 014 [--summary] [--repo PATH]

# Filter by status.
privacytk cfd --status Accepted [--repo PATH]

# Build the session-start manifest: CLAUDE.md + the index +
# CURRENT_STATUS.md's In Progress/Next Priorities + the N most
# recent ADRs. The single artefact an agent should load at /session:start.
privacytk cfd-context [--top-adrs N] [--repo PATH]
```

Both forms of CFD's `_index.md` are auto-detected:

- **Table form** (`| ID | Title | Status | Date |`) — generated by
  adopters who run `cp -R templates/. .`.
- **Bullets form** (`- [Title](path)`) — used by the upstream CFD repo
  itself (`albertomarturelo/context-first-development`).

Privacy properties:

- Pure local reads. The parser never opens a socket, never makes a
  network call, never writes outside the repo you point it at.
- `--id` reads exactly one ADR file. The agent gets the content of the
  one decision it asked for, not the whole `docs/decisions/` folder.
- `--summary` returns a single line so the agent can decide whether
  to load the full ADR — this is the case where privacytk saves the
  most tokens versus the agent reading the file cold.
- `cfd-context` is **opt-in**. CI agents do not need it; a developer
  who wants a session-start bundle runs it once.

Concrete savings, measured against the upstream CFD repo:

| Query | `cat docs/decisions/_index.md` | `privacytk cfd` output |
|---|---|---|
| `repository` | 3.6 KB (full index) | 1 line, ~80 bytes |
| `--id ADR-XXX` | 4-8 KB (full ADR body) | ~3-6 KB (just that ADR) or ~120 bytes (summary) |

Recommended wiring for a `CLAUDE.md` or `AGENTS.md`:

```text
For architectural decisions, run `privacytk cfd <topic>` and read
the returned ADR by id (or summary) — never read `docs/decisions/`
whole. For session start, run `privacytk cfd-context --top-adrs 5`.
```

### Future improvements (not yet implemented)

The current `--top-adrs N` ranking is date-descending. For repos
with many ADRs this returns mostly-irrelevant decisions. Three
planned improvements preserve fidelity by trading a fixed byte
budget for context-aware selection:

1. **Manifest + tools API** (`pkt cfd-context --manifest-and-tools`).
   Bundle emits the manifest plus a JSON tool descriptor so the
   LLM invokes `cfd-read` for ADRs it actually needs instead of
   receiving them inline. Closer to how Claude Code already
   consumes the rest of the system (tools on demand).
2. **Per-ADR importance scoring.** Combine recency, decisiveness
   (`Accepted` > `Proposed` > `Superseded`), and topic relevance
   (heuristic: the last commands the LLM ran) to rank by
   importance, not just date.
3. **Embedding-based similarity.** For repos with 50+ ADRs,
   cosine similarity between the LLM's recent task description
   and each ADR's title/summary picks the top 5. Adds 50–200 ms
   per bundle; only worth it for large decision repositories.

Until these land, the fidelity is preserved by the byte-budget
truncation marker pointing the LLM at the ADR's path, and the
CFD context injector auto-firing before architecturally-
significant commands. See the README FAQ ("Does the CFD
context lose fidelity compared to upstream CFD?" onward) for
the operator-facing descriptions.

## 2d. Automatic hooks (SessionStart + CFD context injector)

A single `pkt init -g --agent claude` installs three hooks
into `~/.claude/settings.json` (and one parallel set per agent you
also init):

| Hook event | Matcher | Command | What it does |
|---|---|---|---|
| `PreToolUse` | `Bash` | `pkt hook claude` | Rewrites `git status` → `pkt git status`, etc. (compression auto-applied) |
| `PreToolUse` | `Bash` | `pkt hook cfd-context-injector` | **CFD auto-injector.** Detects architecturally-significant commands (`git commit -m`, `git checkout -b`, `cargo new`, etc.). Looks up the related ADR via `pkt cfd <topic>` and emits it as `hookSpecificOutput.additionalContext` so the agent sees the decision before running the command. Silent no-op for observation commands (`git status`, `cargo test`). |
| `SessionStart` | `""` | `pkt hook session-start` | Emits the CFD manifest (CLAUDE.md + decisions index + `CURRENT_STATUS.md` In-Progress/Next-Priorities + 5 most-recent ADRs) to stdout. Claude Code prepends it to the session context. |

**The pre-commit audit gate is intentionally not installed.** The
corporate fork assumes a PR-based workflow with human review, and
GHE push protection already blocks pushes that contain common
secrets. Adding a second gate at commit time just generates friction
(falsos positivos en fixtures, tests, etc.) for no additional
safety. `pkt audit <path>` stays available as a manual command
the developer (or a CI step) can run before pushing, but never blocks
a commit.

If a future operator wants the local gate back, it is one flag to
re-enable: change `insert_corporate_hooks` in `src/hooks/init/mod.rs`
to take a third argument and add the corresponding `PreToolUse`
entry. The drift test in `src/core/drift_check.rs` asserts the constant
is *absent* — flip the assertion if you re-enable.

After `init`, the developer does not invoke `pkt` manually
for the common cases:

- **Bash compression** happens automatically on every command the
  LLM tries to run.
- **CFD context** is loaded on session start.
- **CFD lookup** happens automatically before architecturally-significant
  commands.

What stays manual:

- `privacytk audit` (run before `git push` if you want a final scan;
  the operator can wire this into a CI step too).
- `privacytk verify-network` (sanity check after editing
  `~/.config/privacytk/network.toml`).
- `privacytk where-state` (confirm zero disk writes after a session).
- `privacytk network-audit <tool>` (pre-flight check on a new wrapper CLI).
- `privacytk cfd <query>` (one-off ADR lookup outside session-start).
- `privacytk dedup` / `--dedup` (opt-in dedup of identical re-runs).

The hooks are idempotent: re-running `init` is safe; existing
entries are not duplicated.

## 3. Verifying a build before shipping it

Run these four checks on every image. Each is a one-liner:

```bash
# 1. Confirm telemetry is compiled out (must print "(not applicable...)").
privacytk telemetry status

# 2. Confirm the network guard loaded with default_deny=true (must print
#    "default_deny: true" and a non-empty allowlist only if you configured one).
privacytk verify-network api.example.com

# 3. Confirm a sample secret-bearing file is flagged.
mkdir -p /tmp/audit-smoke && \
  echo 'AKIAABCDEFGHIJKLMNOP=AKIAABCDEFGHIJKLMNOP' > /tmp/audit-smoke/env && \
  privacytk audit /tmp/audit-smoke --human
# Expected: exit 1, prints the AWS access key with preview "AKIA…OP (20 chars)".

# 4. Confirm zero outbound sockets during a verify-network run.
privacytk verify-network api.example.com &
PID=$!
sleep 0.05
lsof -p $PID | grep -cE 'IPv4|IPv6'    # must print 0
wait $PID
```

Add the smoke to CI as a release gate. If any of those prints the wrong thing,
block the image.

## 4. Allowlist maintenance

The allowlist is the highest-value file in the deployment. Treat it like
`/etc/hosts`: review changes, version it, sign it.

Recommended workflow:

1. Add a host to `~/.config/privacytk/network.toml` under `allow = […]`.
2. Run `privacytk verify-network <new-host>` and confirm the verdict flips to
   `allow`.
3. Commit the change to the internal IaC repo, signed, with a one-line
   justification in the commit body.
4. Roll out via your config-management tool (Ansible, Chef, Puppet, MDM).

Do **not** rely on a global allowlist in `/etc/skel` — operators must be able
to add hosts without rebuilding the base image. The per-user file at
`~/.config/privacytk/network.toml` is intentional.

## 5. Audit log integration

`privacytk audit` is designed to be the front of a CI gate:

```bash
# In CI, before the agent gets to see a pull request:
privacytk audit path/to/diff > audit-report.json
if [[ $? -ne 0 ]]; then
    # Upload audit-report.json to your GRC system and fail the build.
    exit 1
fi
```

The JSON output is stable and includes:

```json
{
  "scanner": "privacytk",
  "root": "...",
  "hit_count": 17,
  "hits": [
    {
      "file": "src/secrets.rs",
      "line": 42,
      "column": 7,
      "category": "github_token",
      "preview": "ghp_…XX (40 chars)"
    }
  ],
  "policy": {
    "outbound_network": "default-deny",
    "telemetry": "compiled out"
  }
}
```

The `preview` field never contains the full secret — only the first 4 chars,
an ellipsis, the last 2 chars, and the length. Treat the length as sensitive
context (it can confirm a guess).

## 6. Subprocess network exposure (the part privacytk does *not* control)

privacytk's own outbound surface is the `network_guard` allowlist. That
covers every URL privacytk itself would request — in the corporate build
this is exactly *zero* URLs, because telemetry is compiled out and no
filter talks to the network.

**What privacytk does not gate**: the subprocesos it spawns on behalf of
the user. `privacytk git status` runs `git`. `privacytk aws s3 ls` runs
`aws`. `privacytk curl ...` runs `curl`. Each of those processes inherits
the parent's network namespace and can open whatever sockets the host OS
policy permits. The corporate fork does not — and cannot, without a
namespace jail — sandbox arbitrary user-invoked binaries.

For your stack, this means:

| Binary | Default network exposure | Mitigation |
|---|---|---|
| `git` | clone / fetch / push to whatever remote the repo points at | corporate `~/.gitconfig` rewriting `url.*.insteadOf` to the internal mirror |
| `gh copilot` | `api.githubcopilot.com` | egress proxy allowlist; or disable the extension in the corporate image |
| `gh` (other) | `api.github.com` | same; or pin to a self-hosted GitHub Enterprise |
| `acme-jira` (your internal CLI) | `jira.internal.example.com` | corporate egress proxy allowlist; pin the binary in the base image |
| `aws` | any AWS endpoint reachable from the workstation | `aws sso` + `~/.aws/config` with a corporate role |
| `kubectl` / `docker` | corporate control plane | already in the allowlist via your VPN |

The operator's tool for inspecting this is `privacytk network-audit
<cmd>`:

```bash
privacytk network-audit gh copilot suggest "list my open issues"
privacytk network-audit acme-jira list-tickets
```

This runs the supplied argv under a network namespace that has only
loopback (Linux `unshare --net` or macOS `sandbox-exec`). On Linux the
child's connect(2) calls fail with ENETUNREACH; on macOS the sandbox
profile kills any process that touches a network syscall. In either case
the operator gets a report listing the attempt and confirming it was
blocked. **No actual egress happens during the audit** — that is the
whole point.

Use this before adopting any new wrapper CLI. If `privacytk network-audit
<new-tool>` reports any blocked TCP attempt, you now know exactly what
that tool would phone home to. You can then decide whether to allowlist
it at the egress proxy or refuse the wrapper.

## 7. What to watch for after deployment

- **`privacytk verify-network` exit codes are not yet wired.** A failed
  guard is logged and printed but the calling command still exits 0 in
  passthrough paths. If you want deny-fails-the-build semantics, wrap
  `verify-network` in your CI step (the binary exits 0 today; check
  `audit`'s non-zero exit on hits when wiring gates).
- **Updates to the upstream `rtk` tree.** Every upstream merge must
  re-run the four smoke checks above. The `ureq` dependency, the
  `ccusage` npx fallback, and the telemetry POST site are the three
  patterns to grep for in a diff before accepting it.
- **Fork drift.** privacytk tracks upstream at the source level. It is
  not a vendored copy with a patch series; when you pull new upstream
  commits, the fork-specific files (`src/core/network_guard.rs`,
  `src/core/audit.rs`, `core/ephemeral.rs`, `core/network_audit.rs`,
  the `audit`/`verify-network`/`where-state`/`network-audit` arms in
  `main.rs`, and the `Cargo.toml` rename) must be re-applied.

### Drift check automation

Two gates run in CI on every PR. Both must pass.

**Shell script** (`scripts/check-fork-drift.sh [BASE_REF]`):
- diff-greps `ureq::`, `npx ccusage`, and `TELEMETRY_URL`/telemetry POST
  patterns against the upstream side;
- asserts four fork-owned files still exist;
- asserts the fork's distinguishing markers are present.

**Cargo test** (`cargo test --bin privacytk core::drift_check`):
15 assertions over the in-tree source. Runs without `git diff` or a
remote, so it catches the same drift even on developer laptops or in
CI runners that don't have `rtk-ai/rtk` cloned.

The GitHub Actions workflow at
`.github/workflows/fork-ci.yml` runs both gates, builds release, and
executes a synthetic-secrets smoke on `privacytk audit`. No third-party
API keys, no telemetry endpoints, no Docker required.

## 8. Emergency: revoke egress

If an incident response reveals that a host on the allowlist is being abused:

1. Edit `~/.config/privacytk/network.toml` and remove the entry.
2. Run `privacytk verify-network <host>` to confirm the verdict flips to
   `deny`.
3. Notify operators via your standard channel.

If you suspect the binary itself is compromised:

1. Pull the latest signed image.
2. Re-run the four smoke checks from §3.
3. File an incident; the binary's `Cargo.lock` is a deterministic manifest
   that can be diffed against the previous good build.