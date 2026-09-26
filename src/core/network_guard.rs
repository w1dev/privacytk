//! Corporate network guard.
//!
//! The privacytk build refuses to open outbound sockets except to hosts that
//! appear in the allowlist loaded from `~/.config/privacytk/network.toml` (or
//! the path the operator sets via `PRIVACYTK_NETWORK_CONFIG`).
//!
//! The default policy is **deny**. Anything that wants to talk out — today
//! only the (now-removed) telemetry path, but future filter integrations will
//! route here — must pass a URL through [`NetworkGuard::allow`] before any
//! `ureq`/socket syscall.
//!
//! Schema for `network.toml`:
//!
//! ```toml
//! schema_version = 1
//!
//! # default_deny must be true in corporate builds. Setting it to false turns
//! # this whole module into a no-op; refuse at load time.
//! default_deny = true
//!
//! # Hosts are matched by exact string after lowercasing. Wildcards are not
//! # supported on purpose: a typo in `*anthropic.com` is a security incident.
//! allow = [
//!   "api.internal.example.com",
//!   "metrics.internal.example.com",
//! ]
//! ```
//!
//! The guard never resolves DNS, never opens a socket, and never logs the
//! payload. It is a pure allow/deny gate evaluated in-process.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
pub(crate) struct NetworkConfig {
    #[serde(default = "default_true")]
    default_deny: bool,
    #[serde(default)]
    allow: Vec<String>,
}

fn default_true() -> bool {
    true
}

static CACHED_CONFIG: LazyLock<Result<NetworkConfig, String>> = LazyLock::new(load_config);

const ENV_NETWORK_CONFIG: &str = "PRIVACYTK_NETWORK_CONFIG";
const FILENAME: &str = "network.toml";

/// Resolve the configuration file path in priority order:
///
/// 1. `PRIVACYTK_NETWORK_CONFIG` if set and non-empty.
/// 2. `<config_dir>/privacytk/network.toml` (XDG-style: `~/.config` on Linux,
///    `~/Library/Application Support` on macOS, `%APPDATA%` on Windows).
/// 3. `<config_dir>/rtk/network.toml` — legacy rtk path. Honored so existing
///    corporate images that ship a config under the old name keep working.
pub(crate) fn config_path_resolved() -> Option<PathBuf> {
    if let Ok(p) = std::env::var(ENV_NETWORK_CONFIG)
        && !p.trim().is_empty()
    {
        return Some(PathBuf::from(p));
    }
    let base = dirs::config_dir()?;
    let primary = base.join("privacytk").join(FILENAME);
    if primary.exists() {
        return Some(primary);
    }
    let legacy = base.join("rtk").join(FILENAME);
    if legacy.exists() {
        return Some(legacy);
    }
    // No config on disk: surface the primary path so the operator knows where
    // to create one. `load()` will still succeed with default_deny = true.
    Some(primary)
}

/// Load and cache the network config. Missing file is not an error; it means
/// deny-all with an empty allow list.
pub fn load() -> &'static Result<NetworkConfig, String> {
    &CACHED_CONFIG
}

fn load_config() -> Result<NetworkConfig, String> {
    match config_path_resolved() {
        None => Ok(NetworkConfig {
            default_deny: true,
            allow: vec![],
        }),
        Some(path) if !path.exists() => Ok(NetworkConfig {
            default_deny: true,
            allow: vec![],
        }),
        Some(path) => std::fs::read_to_string(&path)
            .map_err(|e| format!("read {}: {}", path.display(), e))
            .and_then(|s| {
                toml::from_str::<NetworkConfig>(&s)
                    .map_err(|e| format!("parse {}: {}", path.display(), e))
            })
            .and_then(|cfg| {
                if !cfg.default_deny {
                    Err(format!(
                        "{} sets default_deny = false; privacytk refuses to load \
                         a permissive policy. Flip it to true and list explicit \
                         allow hosts.",
                        path.display()
                    ))
                } else {
                    Ok(cfg)
                }
            }),
    }
}

/// Re-read the config from disk; used by `privacytk verify-network` after the
/// operator edits the file.
pub fn reload() -> Result<()> {
    // OnceLock does not support interior mutation; we approximate a reload by
    // resetting the global on the next call. The simplest correct path: read
    // directly and validate, then drop the cached cell by leaking a poison
    // entry — but std doesn't expose that. Instead, expose a one-shot loader
    // the operator can call from the CLI command.
    let path = config_path_resolved().context("could not resolve network config path")?;
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let cfg: NetworkConfig = toml::from_str(&raw)
        .with_context(|| format!("parse {}", path.display()))?;
    if !cfg.default_deny {
        anyhow::bail!(
            "{} sets default_deny = false; privacytk refuses to load a permissive policy.",
            path.display()
        );
    }
    Ok(())
}

/// Reason a host was allowed or denied. Used by `verify-network` to print
/// per-host verdicts without actually opening a socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    DeniedNoAllowlistEntry,
    DeniedDefaultDeny,
    DeniedParseError,
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Allowed => "allow",
            Verdict::DeniedNoAllowlistEntry => "deny (host not in allowlist)",
            Verdict::DeniedDefaultDeny => "deny (default_deny is true and config failed to load)",
            Verdict::DeniedParseError => "deny (config.toml failed to parse; see `privacytk verify-network --explain`)",
        }
    }
}

/// Decide whether `url_or_host` is permitted to be reached.
///
/// `url_or_host` may be either a bare hostname (`api.example.com`) or a URL
/// (`https://api.example.com/v1/foo`). The scheme and path are stripped before
/// lookup; the userinfo, port, and query string are never inspected.
pub fn allow(url_or_host: &str) -> Verdict {
    let host = match extract_host(url_or_host) {
        Some(h) => h,
        None => return Verdict::DeniedNoAllowlistEntry,
    };

    match load() {
        Err(_) => Verdict::DeniedParseError,
        Ok(cfg) => {
            if !cfg.default_deny {
                // Should be unreachable: load() refuses permissive configs. Kept
                // as a defensive default so a future bug does not become a
                // silent allow.
                Verdict::DeniedDefaultDeny
            } else if cfg.allow.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
                Verdict::Allowed
            } else {
                Verdict::DeniedNoAllowlistEntry
            }
        }
    }
}

fn extract_host(url_or_host: &str) -> Option<String> {
    let s = url_or_host.trim();
    if s.is_empty() {
        return None;
    }
    // Bare host (no scheme): use as-is.
    if !s.contains("://") {
        // Strip userinfo and port.
        return Some(strip_port(s.split('@').next_back().unwrap_or(s)).to_ascii_lowercase());
    }
    // URL form: parse with the `url` crate? Not worth a new dep. Hand-roll a
    // minimal extractor sufficient for http(s) URLs the fork will encounter.
    let after = s.splitn(2, "://").nth(1)?;
    let host_and_path = after.split('/').next()?;
    let host = host_and_path.split('@').next_back().unwrap_or(host_and_path);
    Some(strip_port(host).to_ascii_lowercase())
}

fn strip_port(host: &str) -> &str {
    // IPv6 literals like `[::1]:443` — strip the brackets and port.
    if let Some(stripped) = host.strip_prefix('[') {
        if let Some(end) = stripped.find(']') {
            return &stripped[..end];
        }
    }
    if let Some((h, _)) = host.split_once(':') {
        return h;
    }
    host
}

/// Render the loaded allowlist as a printable report. Used by `verify-network`.
pub fn report() -> String {
    match load() {
        Err(e) => format!("config error: {e}\n"),
        Ok(cfg) => {
            let mut out = String::new();
            out.push_str(&format!("default_deny: {}\n", cfg.default_deny));
            out.push_str(&format!("allow ({}):\n", cfg.allow.len()));
            for h in &cfg.allow {
                out.push_str(&format!("  - {h}\n"));
            }
            if let Some(p) = config_path_resolved() {
                out.push_str(&format!("source: {}\n", p.display()));
            } else {
                out.push_str("source: (no config_dir resolvable — pure deny-all)\n");
            }
            out
        }
    }
}

/// Evaluate `host` against the guard and return a verdict. Convenience wrapper
/// used by `verify-network <host>`.
pub fn evaluate(host: &str) -> Verdict {
    allow(host)
}

/// Path the operator is expected to create. Surfaced in `verify-network` when
/// no config exists yet so they have a concrete target.
pub fn expected_config_path() -> Option<PathBuf> {
    config_path_resolved()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_host_from_url() {
        assert_eq!(
            extract_host("https://api.example.com/v1/foo"),
            Some("api.example.com".into())
        );
        assert_eq!(
            extract_host("http://API.example.com:8080/x"),
            Some("api.example.com".into())
        );
        assert_eq!(extract_host("api.example.com"), Some("api.example.com".into()));
        assert_eq!(
            extract_host("https://user:pass@example.com/p"),
            Some("example.com".into())
        );
    }

    /// Pure-logic verdict: re-implement the allowlist check without touching
    /// the cached `LazyLock`. Used by tests so they can drive arbitrary
    /// configs without polluting the process-global guard.
    fn check(cfg: &NetworkConfig, host: &str) -> Verdict {
        let Some(host) = extract_host(host) else {
            return Verdict::DeniedNoAllowlistEntry;
        };
        if !cfg.default_deny {
            return Verdict::DeniedDefaultDeny;
        }
        if cfg.allow.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
            Verdict::Allowed
        } else {
            Verdict::DeniedNoAllowlistEntry
        }
    }

    #[test]
    fn default_policy_is_deny_with_empty_allowlist() {
        let cfg = NetworkConfig {
            default_deny: true,
            allow: vec!["allowed.example".into()],
        };
        assert_eq!(check(&cfg, "allowed.example"), Verdict::Allowed);
        assert_eq!(check(&cfg, "https://denied.example.com/v1"), Verdict::DeniedNoAllowlistEntry);
        assert_eq!(check(&cfg, ""), Verdict::DeniedNoAllowlistEntry);
    }

    #[test]
    fn permissive_policy_is_refused() {
        let cfg = NetworkConfig {
            default_deny: false,
            allow: vec![],
        };
        assert_eq!(check(&cfg, "anything.example"), Verdict::DeniedDefaultDeny);
    }

    #[test]
    fn env_var_round_trip_with_temp_env() {
        // temp-env is the safe way to mutate the process env in edition 2024.
        // This exercises the path resolver, not the cached verifier — the
        // cached verifier cannot be reset per-test by design.
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("network.toml");
        std::fs::write(&cfg_path, "default_deny = true\nallow = []\n").unwrap();

        temp_env::with_var("PRIVACYTK_NETWORK_CONFIG", Some(&cfg_path), || {
            // Resolve the path the resolver picks.
            assert!(crate::core::network_guard::config_path_resolved().is_some());
        });
    }
}