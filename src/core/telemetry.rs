//! Telemetry has been removed in the privacytk corporate fork.
//!
//! The upstream `rtk` project optionally POSTs an anonymous usage ping to
//! `TELEMETRY_URL` once per day. Even with consent gating, the binary talking
//! out on its own is incompatible with corporate environments that require
//! zero uninitiated egress and a deny-by-default network policy.
//!
//! This module keeps the public surface (`maybe_ping`, `endpoint_url`,
//! `telemetry_marker_path`, `salt_file_path`, `generate_device_hash`) so the
//! callers in `main.rs`, `telemetry_cmd.rs`, `init`, and the tests continue to
//! link, but every function returns `None` / no-ops / a deterministic hash.
//!
//! See `docs/CORPORATE.md` for the full threat model and the audit story.

use std::path::PathBuf;

const PING_INTERVAL_SECS: u64 = 23 * 3600; // kept for callers that read it

/// Always returns `None` — privacytk has no telemetry endpoint.
pub fn endpoint_url() -> Option<&'static str> {
    None
}

/// No-op. The original implementation spawned a thread that POSTed JSON; here
/// we deliberately do nothing. Callers in `main.rs` keep their firing site so
/// the diff against upstream stays small.
pub fn maybe_ping() {
    // Intentionally empty.
}

/// Deterministic, non-identifying hash. The upstream version salted with a
/// per-installation random 32-byte value persisted to disk and then SHA-256
/// hashed with `salt`. That made the value a stable per-install fingerprint
/// which is itself a privacy concern in shared-image environments.
///
/// We return a fixed placeholder so tests and the `telemetry status` command
/// still have something to print.
pub fn generate_device_hash() -> String {
    // 64 ASCII hex chars (sha256-length placeholder), same shape as upstream.
    "0".repeat(64)
}

/// Salt file path kept only because `telemetry_cmd` calls it for the status
/// line. The file is never written or read.
pub fn salt_file_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("privacytk")
        .join(".device_salt.disabled")
}

/// Marker path kept only because `telemetry_cmd` calls it for the status line.
/// Never written.
pub fn telemetry_marker_path() -> PathBuf {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("privacytk");
    data_dir.join(".telemetry_disabled")
}

/// Kept for binary compatibility with upstream callers that referenced it.
pub const fn ping_interval_secs() -> u64 {
    PING_INTERVAL_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_url_is_none() {
        assert!(endpoint_url().is_none());
    }

    #[test]
    fn device_hash_is_deterministic_placeholder() {
        let a = generate_device_hash();
        let b = generate_device_hash();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn paths_live_under_privacytk_dir() {
        let salt = salt_file_path().to_string_lossy().into_owned();
        let marker = telemetry_marker_path().to_string_lossy().into_owned();
        assert!(salt.contains("privacytk"), "salt path was {salt}");
        assert!(marker.contains("privacytk"), "marker path was {marker}");
    }

    #[test]
    fn maybe_ping_is_a_noop() {
        // Smoke: must not panic, must not return anything (it is `()`).
        maybe_ping();
    }
}