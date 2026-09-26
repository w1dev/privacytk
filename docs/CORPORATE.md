# Corporate deployment guide

This document covers the operational concerns that aren't in the README:
image packaging, policy files, and what to audit when you onboard privacytk
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

## 6. What to watch for after deployment

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
  `src/core/audit.rs`, the `audit` and `verify-network` arms in
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

## 7. Emergency: revoke egress

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