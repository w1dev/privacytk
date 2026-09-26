# privacytk

> Forked from [`rtk-ai/rtk`](https://github.com/rtk-ai/rtk) at
> [v0.49.0](https://github.com/rtk-ai/rtk/releases/tag/v0.49.0).
> privacytk keeps the upstream's compression filters and adds the corporate
> guarantees below. Apache-2.0, inherited from upstream. The
> `forked from` clause is the only place this README names the upstream;
> the binary, subcommands, hooks, and configuration all live under the
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

---

## Table of contents

- [Build](#build)
- [Distribution & delivery](#distribution--delivery)
- [Install](#install)
- [How to test](#how-to-test)
- [Install the network allowlist](#install-the-network-allowlist)
- [Commands](#commands)
- [Threat model](#threat-model)
- [FAQ](#faq)
- [License](#license)

---

## Build

Requires Rust 1.91 or newer (matches `rust-version` in `Cargo.toml`).

```bash
cargo build --release
# binary at target/release/privacytk
```

The toolchain is keg-only under Homebrew; if `cargo` is not on `PATH`, prepend
`/opt/homebrew/opt/rustup/bin`.

`cargo check` and `cargo build` emit roughly 30 dead-code warnings on symbols
inherited from upstream. This is intentional: the fork policy is
`warnings = "warn"` (instead of the upstream `warnings = "deny"`) so a quiet
refactor upstream does not brick the fork. The drift check (§ How to test)
guards against real regressions, not warning churn.

---

## Distribution & delivery

privacytk is delivered as a single static-ish Rust binary. There is no
runtime dependency beyond libc (Linux/macOS) or the Windows system DLLs
that `windows-sys` already links.

### Distribution channels

| Channel | When to use | Build artefact |
|---|---|---|
| **Local developer build** | Quick smoke-test, iterating on the source | `cargo build --release` → `target/release/privacytk` |
| **CI artefact** (preferred) | Reproducible builds for the corporate image | The CI pipeline builds, signs, and uploads per-platform binaries |
| **OS package** | Distribution via apt/dnf/brew within the corp image | `.deb` (cargo-deb) or `.rpm` (cargo-generate-rpm), or Homebrew formula pinned to a tap |

The `cargo-deb` and `cargo-generate-rpm` metadata in `Cargo.toml` produces
the Debian and RPM artefacts without extra configuration. Use them; do not
invent a new packaging pipeline.

### Recommended CI pipeline (per release)

The canonical example lives in `.github/workflows/fork-ci.yml`. The
pipeline:

1. Runs `scripts/check-fork-drift.sh` on the PR diff.
2. Runs `cargo test --bin privacytk --release` (all unit tests).
3. Builds release binaries per OS/arch.
4. Runs the four smoke checks from [How to test](#how-to-test).
5. Uploads binaries + sha256 sums as release artefacts.
6. Publishes to the internal package repository.

### Versioning

- The fork follows semantic versioning.
- A merge from upstream bumps the minor version (the fork keeps the
  upstream filters, so the surface changes are additive).
- Adding a new corporate-only module (`network_guard`, `audit`, etc.)
  bumps the minor version.
- Bug fixes that do not change the public surface bump the patch version.

### Provenance

Every release artefact should be signed. The recommended chain is:

```bash
# 1. Build (CI does this; shown here for local sanity-check).
cargo build --release

# 2. Hash.
sha256sum target/release/privacytk > privacytk.sha256

# 3. Sign (adjust key/cert paths to your PKI).
cosign sign-blob --key cosign.key --output-signature privacytk.sig \
    target/release/privacytk

# 4. Bundle.
tar -czf privacytk-<version>-<target>.tar.gz -C target/release privacytk
```

The operator verifies with `cosign verify-blob --key cosign.pub --signature privacytk.sig privacytk`.

---

## How to test

Three layers of tests. **All three must pass before a release.**

### 1. Drift check (bash, no Rust required)

```bash
scripts/check-fork-drift.sh HEAD~1
```

Greps the diff against the previous commit for forbidden patterns
(`ureq::`, `npx ccusage`, `TELEMETRY_URL`), asserts the four fork-owned
files exist, and asserts the fork's distinguishing markers are present.
This is the test of record for "did a merge reintroduce something we
removed?". POSIX-portable bash 3.2 compatible (no `declare -A`).

### 2. Drift unit tests (cargo, no network)

```bash
cargo test --bin privacytk --release core::drift_check
```

21 source-level invariants: `ureq` is absent from `Cargo.toml`, the telemetry
module has no POST site, ccusage has no `npx` invocation, hooks are wired
via `insert_corporate_hooks`, etc. Runs in CI without external dependencies.

### 3. Full unit test suite

```bash
cargo test --bin privacytk --release
```

All modules: `audit` (7), `cfd` (12), `dedup` (6), `drift_check` (21),
`network_audit` (11), `network_guard` (4), plus the upstream test suite
that still compiles (~3700 tests). Total: **3770+ tests, 0 failures
expected**.

### Smoke battery (post-build, pre-publish)

Run these four checks on every binary that will ship. Each is a
one-liner that catches a class of breakage the unit tests miss.

```bash
BIN=./target/release/privacytk

# 1. Telemetry is compiled out (must NOT print a URL).
$BIN telemetry status
#   expected: "(not applicable; this build has no telemetry endpoint)"

# 2. Network guard default-denies unknown hosts (must NOT print "allow").
$BIN verify-network api.example.com
#   expected: "deny (host not in allowlist)"

# 3. Audit scanner detects a synthetic secret (must show redacted preview).
mkdir -p /tmp/_smoke
printf 'token = ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n' > /tmp/_smoke/t
$BIN audit /tmp/_smoke --human
#   expected: at least one hit, preview "ghp_…AA (40 chars)", no full token
rm -rf /tmp/_smoke

# 4. Zero outbound sockets during a verify-network run.
$BIN verify-network api.example.com &
PID=$!
sleep 0.05
SOCKETS=$(lsof -p $PID 2>/dev/null | grep -cE 'IPv4|IPv6')
wait $PID
test "$SOCKETS" = "0"
#   expected: sockets count is 0
```

If any of those prints the wrong thing, block the release.

### On-disk footprint check

```bash
$BIN where-state
#   expected: a table listing history.db, recall.db, tee/, etc., each
#   with exists/absent and size_bytes. Lets the operator confirm that
#   no state is being persisted that they did not approve.
```

### Manual acceptance test (developer workstation)

After `cargo build --release`:

```bash
# 1. The filter pass works on real output.
$BIN git status                 # compressed output
$BIN cargo --version            # passthrough
$BIN kubectl get pods -A 2>&1 | head  # kubernetes compression (skips if kubectl absent)

# 2. CFD integration parses the upstream CFD repo.
git clone https://github.com/albertomarturelo/context-first-development.git /tmp/cfd
$BIN cfd --repo /tmp/cfd             # full index
$BIN cfd "repository" --repo /tmp/cfd  # one match
$BIN cfd-context --repo /tmp/cfd --top-adrs 3  # session-start bundle

# 3. Ephemeral mode touches no FS.
PRIVACYTK_EPHEMERAL=1 $BIN ls .     # runs, ~/.local/share/privacytk/ unchanged

# 4. Audit finds a planted secret.
mkdir -p /tmp/_audit_smoke
echo 'AWS_ACCESS_KEY_ID=AKIAABCDEFGHIJKLMNOP' > /tmp/_audit_smoke/env
$BIN audit /tmp/_audit_smoke --human
rm -rf /tmp/_audit_smoke
```

---

## Install

Three installation paths. Pick the one that matches your role.

### Path 1 — Developer (you, on your laptop, want to try it)

The simplest path is the bundled installer. From a clone of this repo:

```bash
./install.sh --verify          # build from source, install to ~/.local/bin, run smoke battery
# or, for Claude Code hooks in the same run:
./install.sh --verify --agent claude
```

The script auto-detects OS and arch, requires Rust 1.91+, builds the
release binary, and installs it. Pass `--user` to force `~/.local/bin`
or `--system` to force `/usr/local/bin` (root-only).

If you do not have a clone, the script also accepts a git URL via
`PRIVACYTK_REPO=https://your-internal-mirror/privacytk.git ./install.sh`.
For developers who want the manual path:

```bash
# 1. Get Rust 1.91+ if you don't have it (skip if you already do).
brew install rustup        # macOS
# or: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Linux
rustup default 1.91.0

# 2. Clone and build.
git clone https://github.com/<your-org>/privacytk.git
cd privacytk
cargo build --release

# 3. Install to a directory on PATH.
install -m 0755 target/release/privacytk ~/.local/bin/privacytk   # macOS / Linux
# or: sudo cp target/release/privacytk /usr/local/bin/privacytk

# 4. Verify.
privacytk --version
# expected: privacytk 0.1.0
privacytk telemetry status | grep 'this build'
# expected: "this build has no telemetry endpoint"
```

The first run writes nothing to `~/.local/share/privacytk/` until you
invoke a command that touches the tracking DB. Run with
`PRIVACYTK_EPHEMERAL=1` to keep your laptop clean.

### Path 2 — Claude Code on your workstation

After Path 1 completes:

```bash
privacytk init -g --agent claude --auto-patch
```

This installs two hooks into `~/.claude/settings.json`:

- `PreToolUse` (`Bash`): rewrites `git status` → `privacytk git status`
  (compression auto-applied to every Bash command the agent runs).
- `SessionStart` (`""`): emits the CFD context bundle at session start
  so the agent opens with the right context already loaded.

Restart Claude Code. To verify:

```bash
cat ~/.claude/settings.json | grep -E 'command":' | sort -u
# expected:
#   "command": "privacytk hook claude"
#   "command": "privacytk hook session-start"
```

Open a Claude Code session and ask anything about your repo. The agent's
first reply will reflect the CFD context that was injected automatically.

### Path 3 — Corporate image (the path that scales)

Build from source in CI. Do not distribute pre-built binaries without
signing.

```dockerfile
FROM your-base:tag
ARG PRIVACYTK_COMMIT=<git-sha-of-release-tag>

# 1. Install Rust toolchain.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain 1.91.0 --profile minimal

# 2. Build from source (CI must run this with the tag pinned).
RUN git clone https://internal.example.com/privacytk.git /src && \
    cd /src && git checkout "$PRIVACYTK_COMMIT" && \
    /root/.cargo/bin/cargo build --release && \
    install -m 0755 /src/target/release/privacytk /usr/local/bin/privacytk

# 3. Drop the deny-all allowlist into /etc/skel so every user
#    inherits the default-deny starting state.
COPY network.toml /etc/skel/.config/privacytk/network.toml

# 4. (Optional) Install the Claude Code hooks globally so every
#    developer's first `git status` is compressed automatically.
RUN privacytk init -g --agent claude --auto-patch
# Note: this writes to /root/.claude, which only the build user sees.
# Per-developer init happens on their first Claude Code session.
```

### Verifying the install

After any of the three paths, run the four-smoke battery from
[How to test](#how-to-test):

```bash
BIN=privacytk   # or ./target/release/privacytk, /usr/local/bin/privacytk, etc.
$BIN telemetry status | grep 'this build'       # compiled out
$BIN verify-network api.example.com             # deny
$BIN audit /tmp/_smoke --human                   # detect with redacted preview
lsof -p $(${BIN} verify-network api.example.com & echo $!) | grep -c IPv   # 0 sockets
```

All four must pass. If any returns the wrong thing, the install is broken —
do not promote the artefact.

### Updating

The fork follows semantic versioning. To update an existing install:

```bash
# 1. Pull the new tag.
cd /path/to/privacytk
git fetch --tags
git checkout v0.<next>.<bugfix>

# 2. Rebuild and replace.
cargo build --release
install -m 0755 target/release/privacytk ~/.local/bin/privacytk

# 3. Re-run the drift script against the previous tag to verify the
#    diff is clean (no reintroduced ureq / ccusage / telemetry POST).
bash scripts/check-fork-drift.sh v0.<prev>.<bugfix>
```

`privacytk init -g` is idempotent — re-running it with a new binary does
not duplicate hooks.

### Uninstalling

```bash
# 1. Remove the hooks (Claude Code example; adapt for your agent).
privacytk init -g --agent claude --uninstall

# 2. Remove the binary.
rm ~/.local/bin/privacytk   # or sudo rm /usr/local/bin/privacytk

# 3. Remove user state (optional; only if you do not want the DB to
#    survive the uninstall).
rm -rf ~/.local/share/privacytk/
rm -rf ~/.config/privacytk/
```

The drift check + the smoke battery are the recommended post-uninstall
verification.

---

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

---

## Commands

### Existing commands

All of the upstream CLI surface works unchanged: `git`, `cargo`, `gh`, `glab`, `aws`,
`docker`, `kubectl`, `ls`, `grep`, `find`, `read`, `err`, `test`, `json`,
`deps`, `env`, `log`, `diff`, `summary`, `smart`, etc. See the upstream
README (linked at the top of this file) for the full table — only the names
`privacytk init` and `privacytk hook` require per-agent path rewrites that
still target the same hooks directory.

### New in the fork

| Command | Purpose |
|---|---|
| `privacytk audit <path> [--human]` | Local secrets/PII scanner with redacted previews. |
| `privacytk verify-network [host ...]` | Inspect the network allowlist without opening sockets. |
| `privacytk where-state [--purge] [--json]` | List every path privacytk can touch on disk. |
| `privacytk network-audit <cmd>` | Run `<cmd>` under a network sandbox; report blocked attempts. |
| `privacytk cfd <query> [--id X] [--status S] [--summary]` | CFD-aware ADR lookup. |
| `privacytk cfd-context [--top-adrs N]` | Build the CFD session-start bundle. |
| `privacytk dedup [--on \| --off]` | Manage the dedup cache. |
| `privacytk telemetry status` | Confirm telemetry is compiled out. |

#### `privacytk audit <path> [--human]`

Walks `<path>` (default `.`) and scans every regular file for matches against
the pattern table in `src/core/audit.rs`. Files starting with `.git/`, symlinks,
and binary files (NUL byte heuristic) are skipped.

- Default: JSON output to stdout, suitable for piping into a CI report.
- `--human`: human-readable lines `path:line:col  category  redacted-preview`.
- Exit code: `0` when no hits, `1` when at least one hit. Never blocks a commit
  (the corporate fork assumes PR + human review; see "What privacytk does
  not address" below).

#### `privacytk verify-network [host ...] [--reload]`

Loads the network policy and prints:

- the resolved config path,
- `default_deny` value,
- the allowlist,
- per-host verdicts for every host or URL passed as an argument.

`--reload` re-reads the config from disk before evaluating. **No socket is
opened even when hosts are passed** — the verdict is computed purely against
the allowlist.

#### `privacytk telemetry status|enable|disable|forget`

`enable` no longer transmits anything; it asks for an acknowledgement so an
audit log can record the operator intent. `forget` deletes the (already
empty) salt file, the marker file, and the local tracking DB.

### Auto-installed hooks

`privacytk init -g --agent <agent>` installs hooks into the agent's
settings file. Two hooks ship by default:

| Hook | Matcher | Effect |
|---|---|---|
| `PreToolUse` | `Bash` | Rewrites `git status` → `privacytk git status`, etc. (compression auto-applied) |
| `SessionStart` | `""` | Emits the CFD context bundle at session start |

No pre-commit gate is installed by default. The corporate fork assumes a
PR-based workflow with human review and relies on GHE push protection for
secret scanning at push time.

---

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
  with one from another source, the hook will execute that code. **Pin your
  binary in your image.**

---

## FAQ

### How is this different from `rtk-ai/rtk`?

**`rtk` is a token-saver. `privacytk` is a token-saver that also does not
report anything about your codebase to anyone, including itself.**

Both compress command output identically. The fork removes the upstream
telemetry POST and the `npx ccusage` fallback, adds a default-deny network
guard, a local secrets/PII scanner, an ephemeral mode, and a CFD integration.
If your only motivation is "save tokens", `rtk` is enough. If your
organization has a written policy against binaries that report usage to
external servers, `privacytk` is what you need.

### Does this lose agent context or decision quality?

Three losses, all trade-offs and all mitigable:

| Loss | When it bites | Mitigation |
|---|---|---|
| **No per-session history in `--ephemeral`** | Long sessions where the LLM needs "what did we run 5 minutes ago" | Default off. Use ephemeral for air-gapped sessions, default mode for long ones. The developer has the terminal visible regardless. |
| **Audit false positives (regex-generous patterns)** | Repos with test fixtures that contain fake PANs / emails / IPs | Document in `CLAUDE.md` that hits in `tests/fixtures/` are informational. Future round: custom ignore lists. |
| **`--dedup` can hide a 1-byte change** | `kubectl describe` timestamps, very rare | `--dedup` is opt-in, never default. For sensitive commands run `--no-dedup`. |

The filter pass itself (what the LLM sees as the compressed output) is
**identical** to the upstream `rtk`. The fork does not degrade compression.

### Should I enable `--ephemeral` by default?

**Yes**, in the corporate image. The cost (no per-session history across
process restarts) is acceptable because:

1. The developer has the terminal and the agent's own context visible.
2. Corporate workstations restart at most a few times per day.
3. The DB in default mode (`~/.local/share/privacytk/`) accumulates over
   months and is rarely cleaned up — that is its own compliance risk.

Use `--ephemeral` by default in the corp image. The developer opts into
default mode (no flag, no env) only if they explicitly need cross-session
history.

### Should I enable `--dedup` by default?

**No.** `--dedup` is a power-user flag. Default off. Enable when the developer
notices they are running the same `cargo test` 5 times in a row and wants
to save the duplication. Disable for commands where a 1-byte change matters
(`kubectl describe`, `git diff --stat`).

### The audit scanner shows hundreds of hits in `tests/fixtures/`. Is that a bug?

**No, it is by design.** The regex table is high-recall on purpose. The
developer reads the hits, identifies the false positives (fixtures, sample
data, expected values in tests), and decides what is real. The audit
never blocks a commit; it only informs. If your agent is acting on
audit hits and proposing bad fixes, document the fixture policy in
`CLAUDE.md` (see the FAQ on context loss above).

### Can I add my own secret patterns?

**Not yet.** The pattern table is hardcoded in `src/core/audit.rs`. Adding
a `~/.config/privacytk/audit.toml` TOML override is a planned follow-up.
Today, if you need a custom pattern, edit the table and rebuild. The
fork is happy to take PRs adding well-known patterns (GitLab `glpat-*`,
Bitbucket `BBTC-*`, your internal CLI's token format, etc.).

### How do I know the binary is the official build and not tampered?

Verify the SHA-256 against the published artefact, then verify the
cosign signature. The release artefact includes both:

```bash
sha256sum -c privacytk.sha256
cosign verify-blob --key cosign.pub --signature privacytk.sig privacytk
```

If your operator does not sign the binary yet, do not ship to production.
The drift check (§ How to test) is a secondary guard: it asserts the
forbidden patterns are absent from source, but it cannot catch a tampered
binary.

### Why is `ureq` removed from `Cargo.toml`?

Because `ureq` is the HTTP client the upstream telemetry uses to POST
usage data. If we removed the telemetry code but kept `ureq`, a future
refactor could accidentally re-introduce a network call. Removing the
crate entirely is a defense-in-depth measure: it is now impossible for
the binary to make an HTTP request by accident.

The fork's own outbound HTTP, if ever added, would route through
`ureq` again — but only after an explicit allowlist lookup. Today
nothing in the fork makes outbound HTTP requests.

### Where does the binary write to disk?

Run `privacytk where-state` to see the live table. In default mode:

- `~/.local/share/privacytk/history.db` (+ `-wal`, `-shm`)
- `~/.local/share/privacytk/recall.db` (+ `-wal`, `-shm`)
- `~/.local/share/privacytk/tee/` (rollback buffer, per-command)
- `~/.config/privacytk/config.toml`, `network.toml`, `filters.toml`
- `~/.local/share/privacytk/.telemetry_disabled` (placeholder, never written)
- `~/.local/share/privacytk/.device_salt.disabled` (placeholder, never written)
- `~/.local/share/privacytk/.hook_warn_last` (mtime marker)

In `--ephemeral` mode: zero bytes in `~/.local/share/privacytk/`. The
tracking DB and recall DB open as `:memory:` SQLite; the tee buffer
goes to a temp dir removed at process exit.

### Why is `init -g` not installing a pre-commit audit gate?

The corporate fork assumes a **PR-based workflow with human review**. The
human reviewer is the secret gate. Adding a second automated gate at
commit time generates friction (false positives on test fixtures, on
sample data, on documentation snippets) for no additional safety. The
audit stays available as a manual command (`privacytk audit <path>`) and
can be wired into a CI step. To re-enable the gate, see `CORPORATE.md` §2d.

### Does this work without Claude Code?

Yes. `privacytk init -g` supports 11 agents: Claude Code, Cursor, Codex,
Gemini CLI, Trae, Pi, OMP, Hermes, Factory Droid, Mistral Vibe, and
GitHub Copilot (CLI mode). Each gets its own per-agent install command
(`privacytk init -g --agent <agent>`). The compression filters and the
CFD integration work without any agent installed — `privacytk git
status` works on its own.

---

## License

Apache-2.0, inherited from the upstream project. See `LICENSE`.