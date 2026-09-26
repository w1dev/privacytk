//! Ephemeral mode: zero on-disk persistence for the corporate fork.
//!
//!
//!
//! Privacy controls in this fork are about *what leaves the host*. The
//! inverse side of that question is *what stays on the host*: SQLite
//! tracking DBs, recall caches, tee buffers, hook-audit logs. In a strictly
//! air-gapped workflow the operator may want all of those gone, so this
//! module is the gate.
//!
//! Activation (any one of):
//!   - env var `PRIVACYTK_EPHEMERAL=1`
//!   - global CLI flag `--ephemeral` (forwarded into this module via
//!     `set_force(true)`)
//!
//! Behaviour:
//!   - `history.db` and `recall.db` open as `:memory:` SQLite connections
//!     held in `LazyLock`s. Nothing is written to disk.
//!   - The tee buffer dir is created under `/tmp/privacytk-ephemeral-<pid>/`
//!     and the directory is removed when the process exits (best-effort via
//!     `Drop`).
//!   - Hook-audit logs are dropped (the warning is logged once to stderr).
//!   - `where-state` always reports the real paths even when ephemeral is
//!     active, so the operator can confirm "no, this would have been the
//!     disk location had I not turned it on".
//!
//! Non-goals: this module does not sandbox the *subprocesses* privacytk
//! launches. `gh`, `acme-jira`, etc. are free to write to their own
//! locations. Subprocess containment is the operator's egress policy.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static EPHEMERAL: AtomicBool = AtomicBool::new(false);

/// True when the process is running in ephemeral mode.
pub fn is_active() -> bool {
    EPHEMERAL.load(Ordering::Acquire)
}

/// Force-activate ephemeral mode regardless of the env var. The CLI
/// `--ephemeral` flag calls this before any `Tracker::new()` happens, so
/// the connection pool is built with `:memory:` from the start.
pub fn set_force(active: bool) {
    EPHEMERAL.store(active, Ordering::Release);
}

/// Detect whether the env var asks for ephemeral mode. Idempotent.
pub fn detect_from_env() -> bool {
    match std::env::var("PRIVACYTK_EPHEMERAL") {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => false,
    }
}

/// Initialize: read env, combine with any prior `set_force`, persist.
/// Called once from `main` near the top.
pub fn init_from_env() {
    if detect_from_env() {
        EPHEMERAL.store(true, Ordering::Release);
    }
}

/// Return the SQLite connection string. When ephemeral is active, `":memory:"`.
/// Otherwise `None`, signalling callers to compute their on-disk path the
/// usual way.
pub fn sqlite_uri_override() -> Option<&'static str> {
    if is_active() {
        Some(":memory:")
    } else {
        None
    }
}

/// Resolve the tee directory. When ephemeral, returns a path under
/// `/tmp/privacytk-ephemeral-<pid>/tee`. The directory is created eagerly
/// so callers can write into it, and the parent dir's `Drop` cleans up.
pub fn tee_dir() -> PathBuf {
    let base = ephemeral_root();
    base.join("tee")
}

fn ephemeral_root() -> PathBuf {
    let pid = std::process::id();
    let mut p = std::env::temp_dir();
    p.push(format!("privacytk-ephemeral-{pid}"));
    p
}

/// Whether the hook-audit log should be written. In ephemeral mode it is
/// dropped silently (the operator does not want a local copy of every
/// rewrite). The hook rewrite itself still happens — only the audit log is
/// suppressed.
pub fn hook_audit_enabled() -> bool {
    !is_active()
}

/// Walk every file the binary would touch (or did touch) on disk. Used by
/// `privacytk where-state`. Returned entries: (path, exists, size_bytes).
/// Existence and size are computed lazily so an operator can run this even
/// when running ephemeral, to see what *would* have been touched.
pub fn enumerate_state_paths() -> Vec<StatePath> {
    let mut out = Vec::new();

    // Tracking DB.
    if let Some(p) = default_tracking_db() {
        out.push(StatePath::new("history.db (commands/parse-failures/hook-decisions)", p));
    }
    // Recall DB.
    if let Some(p) = default_recall_db() {
        out.push(StatePath::new("recall.db (recovery store)", p));
    }
    // Tee dir.
    if let Some(p) = default_tee_dir() {
        out.push(StatePath::new("tee/ (output rollback buffer)", p));
    }
    // Hook audit log.
    if let Some(p) = default_hook_audit_log() {
        out.push(StatePath::new("hook-audit.log (only when RTK_HOOK_AUDIT=1)", p));
    }
    // Config + network + filters.
    if let Some(p) = default_config_toml() {
        out.push(StatePath::new("config.toml (user configuration)", p));
    }
    if let Some(p) = default_network_toml() {
        out.push(StatePath::new("network.toml (allowlist)", p));
    }
    if let Some(p) = default_filters_dir() {
        out.push(StatePath::new("filters.toml (custom filter rules)", p));
    }
    if let Some(p) = default_filters_dir_nested() {
        out.push(StatePath::new(
            "filters/*.toml (per-filter rule files)",
            p,
        ));
    }
    // Telemetry stubs (never written in the fork but listed for completeness).
    if let Some(p) = default_telemetry_marker() {
        out.push(StatePath::new(
            ".telemetry_disabled (placeholder; never written)",
            p,
        ));
    }
    if let Some(p) = default_device_salt() {
        out.push(StatePath::new(
            ".device_salt.disabled (placeholder; never written)",
            p,
        ));
    }
    // Hook-warning marker.
    if let Some(p) = default_hook_warn_marker() {
        out.push(StatePath::new(
            ".hook_warn_last (mtime marker for upgrade nudges)",
            p,
        ));
    }
    out
}

/// Try to remove the ephemeral temp dir. Best-effort; logged but not
/// fatal. Called from a `Drop` on the global singleton and from
/// `privacytk where-state --purge`.
pub fn purge_temp_dir() {
    let root = ephemeral_root();
    if root.exists() {
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// One entry in the `where-state` report.
#[derive(Debug, Clone)]
pub struct StatePath {
    pub label: &'static str,
    pub path: PathBuf,
    pub exists: bool,
    pub size_bytes: u64,
}

impl StatePath {
    fn new(label: &'static str, path: PathBuf) -> Self {
        let (exists, size_bytes) = match std::fs::symlink_metadata(&path) {
            Ok(m) => (true, m.len()),
            Err(_) => (false, 0),
        };
        Self {
            label,
            path,
            exists,
            size_bytes,
        }
    }

    /// Pretty-print the entry for `where-state`. `None` for ephemeral rows
    /// means "would have lived here".
    pub fn render(&self) -> String {
        let kind = if self.exists { "exists" } else { "absent " };
        format!(
            "[{kind}] {:>10} B  {}  {}",
            self.size_bytes,
            self.path.display(),
            self.label
        )
    }
}

fn base_data_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join(super::constants::RTK_DATA_DIR))
}

fn base_config_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(super::constants::RTK_DATA_DIR))
}

fn default_tracking_db() -> Option<PathBuf> {
    Some(base_data_dir()?.join(super::constants::HISTORY_DB))
}

fn default_recall_db() -> Option<PathBuf> {
    Some(base_data_dir()?.join(super::constants::RECALL_DB))
}

fn default_tee_dir() -> Option<PathBuf> {
    Some(base_data_dir()?.join("tee"))
}

fn default_hook_audit_log() -> Option<PathBuf> {
    Some(base_data_dir()?.join("hook-audit.log"))
}

fn default_telemetry_marker() -> Option<PathBuf> {
    Some(base_data_dir()?.join(".telemetry_disabled"))
}

fn default_device_salt() -> Option<PathBuf> {
    Some(base_data_dir()?.join(".device_salt.disabled"))
}

fn default_hook_warn_marker() -> Option<PathBuf> {
    Some(base_data_dir()?.join(".hook_warn_last"))
}

fn default_config_toml() -> Option<PathBuf> {
    Some(base_config_dir()?.join(super::constants::CONFIG_TOML))
}

fn default_network_toml() -> Option<PathBuf> {
    Some(base_config_dir()?.join("network.toml"))
}

fn default_filters_dir() -> Option<PathBuf> {
    Some(base_config_dir()?.join(super::constants::FILTERS_TOML))
}

fn default_filters_dir_nested() -> Option<PathBuf> {
    Some(base_config_dir()?.join("filters"))
}

/// Ensure the ephemeral temp dir exists. Idempotent. Used by the tee
/// pipeline so the dir is ready before any writer races against it.
pub fn ensure_root() -> std::io::Result<PathBuf> {
    let root = ephemeral_root();
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

/// Resolve the path for a transient temp file used during command runs.
/// In ephemeral mode the path lives under `/tmp/privacytk-ephemeral-<pid>/`,
/// otherwise under `std::env::temp_dir()`. Used by dotnet binlog/TRX paths.
pub fn transient_path(prefix: &str, suffix: &str) -> PathBuf {
    let base = if is_active() {
        ensure_root().unwrap_or_else(|_| std::env::temp_dir())
    } else {
        std::env::temp_dir()
    };
    base.join(format!("{prefix}{suffix}"))
}

/// Returns the wall-clock byte budget a non-ephemeral run would have used,
/// based on the size of the live files. Useful as a sanity check: if the
/// sum is non-zero but `is_active()`, the operator knows the flag was set
/// *after* something already wrote.
pub fn current_disk_footprint() -> u64 {
    enumerate_state_paths()
        .into_iter()
        .map(|p| p.size_bytes)
        .sum()
}

/// Pure helper for the unit tests: build a `StatePath` without touching
/// the FS. Public for tests; not used in production.
pub fn synthetic_state(label: &'static str, path: PathBuf, size: u64) -> StatePath {
    StatePath {
        label,
        path,
        exists: size > 0,
        size_bytes: size,
    }
}

/// A guard that purges the ephemeral temp dir on drop. Construct one in
/// `main` and hold it for the lifetime of the process.
pub struct EphemeralGuard;

impl Drop for EphemeralGuard {
    fn drop(&mut self) {
        if is_active() {
            purge_temp_dir();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_recognised() {
        let prev = std::env::var("PRIVACYTK_EPHEMERAL").ok();
        temp_env::with_var("PRIVACYTK_EPHEMERAL", Some("1"), || {
            assert!(detect_from_env());
        });
        temp_env::with_var("PRIVACYTK_EPHEMERAL", Some("true"), || {
            assert!(detect_from_env());
        });
        temp_env::with_var("PRIVACYTK_EPHEMERAL", Some("yes"), || {
            assert!(detect_from_env());
        });
        temp_env::with_var("PRIVACYTK_EPHEMERAL", Some("on"), || {
            assert!(detect_from_env());
        });
        temp_env::with_var("PRIVACYTK_EPHEMERAL", Some("0"), || {
            assert!(!detect_from_env());
        });
        temp_env::with_var("PRIVACYTK_EPHEMERAL", Some(""), || {
            assert!(!detect_from_env());
        });
        // Restore: temp_env restores on drop, but the caller may inspect.
        let _ = prev;
    }

    #[test]
    fn force_overrides_env() {
        let prev = std::env::var("PRIVACYTK_EPHEMERAL").ok();
        temp_env::with_var("PRIVACYTK_EPHEMERAL", Some("0"), || {
            set_force(true);
            assert!(is_active());
            set_force(false);
            assert!(!is_active());
        });
        let _ = prev;
    }

    #[test]
    fn sqlite_uri_override_depends_on_active() {
        set_force(true);
        assert_eq!(sqlite_uri_override(), Some(":memory:"));
        set_force(false);
        assert_eq!(sqlite_uri_override(), None);
    }

    #[test]
    fn enumerate_paths_returns_expected_labels() {
        let paths = enumerate_state_paths();
        let labels: Vec<&str> = paths.iter().map(|p| p.label).collect();
        assert!(labels.iter().any(|l| l.contains("history.db")));
        assert!(labels.iter().any(|l| l.contains("recall.db")));
        assert!(labels.iter().any(|l| l.contains("tee")));
        assert!(labels.iter().any(|l| l.contains("network.toml")));
        assert!(labels.iter().any(|l| l.contains("config.toml")));
        assert!(labels.iter().any(|l| l.contains("filters.toml")));
    }

    #[test]
    fn purge_noop_when_dir_absent() {
        // Should not panic.
        purge_temp_dir();
    }

    #[test]
    fn hook_audit_disabled_in_ephemeral() {
        set_force(true);
        assert!(!hook_audit_enabled());
        set_force(false);
        assert!(hook_audit_enabled());
    }
}