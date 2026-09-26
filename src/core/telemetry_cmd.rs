use anyhow::{Context, Result};
use clap::Subcommand;

const TELEMETRY_DISABLED_ENV: &str = "PRIVACYTK_TELEMETRY_DISABLED";
/// Kept for source compatibility with upstream tooling that referenced the
/// original `RTK_TELEMETRY_DISABLED=1` value. We honor both spellings.
const TELEMETRY_DISABLED_ENV_LEGACY: &str = "RTK_TELEMETRY_DISABLED";
const TELEMETRY_DISABLED_VALUE: &str = "1";

/// Label for the `device hash` row when no salt file exists.
fn salt_missing_label(
    endpoint_configured: bool,
    env_override: bool,
    enabled: bool,
    consent_given: Option<bool>,
) -> &'static str {
    if !endpoint_configured {
        return "(not applicable; this build has no telemetry endpoint)";
    }
    if env_override {
        return "(blocked by PRIVACYTK_TELEMETRY_DISABLED)";
    }
    match consent_given {
        Some(false) | None => "(telemetry not enabled; run `privacytk telemetry enable` to opt in)",
        Some(true) if !enabled => "(telemetry disabled in config.toml)",
        Some(true) => "(no salt file yet; written on the first ping)",
    }
}

fn device_hash_line(
    endpoint_configured: bool,
    env_override: bool,
    enabled: bool,
    consent_given: Option<bool>,
    hash: Option<&str>,
) -> String {
    match hash {
        Some(h) if is_device_hash(h) => {
            format!("  device hash:   {}...{}", &h[..8], &h[56..])
        }
        Some(_) => "  device hash:   (malformed device hash)".to_string(),
        None => format!(
            "  device hash:   {}",
            salt_missing_label(endpoint_configured, env_override, enabled, consent_given)
        ),
    }
}

fn is_device_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Subcommand)]
pub enum TelemetrySubcommand {
    Status,
    Enable,
    Disable,
    Forget,
}

pub fn run(command: &TelemetrySubcommand) -> Result<()> {
    match command {
        TelemetrySubcommand::Status => run_status(),
        TelemetrySubcommand::Enable => run_enable(),
        TelemetrySubcommand::Disable => run_disable(),
        TelemetrySubcommand::Forget => run_forget(),
    }
}

/// Returns true when telemetry is explicitly disabled through either the new
/// (`PRIVACYTK_TELEMETRY_DISABLED=1`) or the legacy (`RTK_TELEMETRY_DISABLED=1`)
/// env var.
pub fn telemetry_disabled_by_env() -> bool {
    let v = |k| std::env::var(k).unwrap_or_default() == TELEMETRY_DISABLED_VALUE;
    v(TELEMETRY_DISABLED_ENV) || v(TELEMETRY_DISABLED_ENV_LEGACY)
}

fn run_status() -> Result<()> {
    let config = crate::core::config::Config::load().unwrap_or_default();

    let consent_str = match config.telemetry.consent_given {
        Some(true) => "yes",
        Some(false) => "no",
        None => "never asked",
    };

    let enabled_str = if config.telemetry.enabled {
        "yes"
    } else {
        "no"
    };

    let env_override = telemetry_disabled_by_env();

    println!("Telemetry status:");
    println!("  consent:       {}", consent_str);
    if let Some(date) = &config.telemetry.consent_date {
        println!("  consent date:  {}", date);
    }
    println!("  enabled:       {}", enabled_str);
    if env_override {
        println!("  env override:  PRIVACYTK_TELEMETRY_DISABLED=1 (blocked)");
    }

    let salt_path = super::telemetry::salt_file_path();
    let hash = if salt_path.exists() {
        Some(super::telemetry::generate_device_hash())
    } else {
        None
    };
    println!(
        "{}",
        device_hash_line(
            super::telemetry::endpoint_url().is_some(),
            env_override,
            config.telemetry.enabled,
            config.telemetry.consent_given,
            hash.as_deref(),
        )
    );

    println!();
    println!("privacytk is a corporate fork: telemetry is compiled out.");
    println!("This build will never POST usage data, even if `enabled = true`.");

    Ok(())
}

fn run_enable() -> Result<()> {
    use std::io::{self, BufRead, IsTerminal};

    if !io::stdin().is_terminal() {
        anyhow::bail!(
            "consent requires interactive terminal — cannot enable telemetry in piped mode"
        );
    }

    eprintln!("privacytk is a corporate fork and does not transmit telemetry.");
    eprintln!("Enabling is a no-op kept for source compatibility with rtk tooling.");
    eprintln!();
    eprint!("Acknowledge and continue? [y/N] ");

    let stdin = io::stdin();
    let mut line = String::new();
    stdin
        .lock()
        .read_line(&mut line)
        .context("Failed to read user input")?;

    let accepted = {
        let response = line.trim().to_lowercase();
        response == "y" || response == "yes"
    };

    crate::hooks::init::save_telemetry_consent(accepted)?;

    if accepted {
        println!("Acknowledged. privacytk still transmits nothing.");
    } else {
        println!("Acknowledgement not recorded.");
    }

    Ok(())
}

fn run_disable() -> Result<()> {
    crate::hooks::init::save_telemetry_consent(false)?;
    println!("Telemetry disabled.");
    Ok(())
}

fn run_forget() -> Result<()> {
    crate::hooks::init::save_telemetry_consent(false)?;

    let salt_path = super::telemetry::salt_file_path();
    let marker_path = super::telemetry::telemetry_marker_path();

    if salt_path.exists() {
        std::fs::remove_file(&salt_path)
            .with_context(|| format!("Failed to delete {}", salt_path.display()))?;
    }

    if marker_path.exists() {
        let _ = std::fs::remove_file(&marker_path);
    }

    let db_path = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(super::constants::RTK_DATA_DIR)
        .join(super::constants::HISTORY_DB);
    if db_path.exists() {
        match std::fs::remove_file(&db_path) {
            Ok(()) => println!("Local tracking database deleted: {}", db_path.display()),
            Err(e) => eprintln!("privacytk: could not delete {}: {}", db_path.display(), e),
        }
    }

    println!("Local telemetry data deleted. Telemetry disabled.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_disabled_by_env_honors_opt_out() {
        temp_env::with_var_unset(TELEMETRY_DISABLED_ENV, || {
            temp_env::with_var_unset(TELEMETRY_DISABLED_ENV_LEGACY, || {
                assert!(
                    !telemetry_disabled_by_env(),
                    "unset env must not count as disabled"
                );
            });
        });

        temp_env::with_var(
            TELEMETRY_DISABLED_ENV,
            Some(TELEMETRY_DISABLED_VALUE),
            || {
                assert!(telemetry_disabled_by_env());
            },
        );

        temp_env::with_var_unset(TELEMETRY_DISABLED_ENV, || {
            temp_env::with_var(
                TELEMETRY_DISABLED_ENV_LEGACY,
                Some(TELEMETRY_DISABLED_VALUE),
                || {
                    assert!(
                        telemetry_disabled_by_env(),
                        "legacy RTK_TELEMETRY_DISABLED=1 must still disable telemetry"
                    );
                },
            );
        });

        for other in ["0", "true", "false", "yes", "no", ""] {
            temp_env::with_var(TELEMETRY_DISABLED_ENV, Some(other), || {
                temp_env::with_var_unset(TELEMETRY_DISABLED_ENV_LEGACY, || {
                    assert!(
                        !telemetry_disabled_by_env(),
                        "value {other:?} must not be treated as disabled"
                    );
                });
            });
        }
    }

    const SAMPLE_HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn salt_missing_label_distinguishes_consent_not_given_from_real_failure() {
        let not_enabled = "(telemetry not enabled; run `privacytk telemetry enable` to opt in)";
        assert_eq!(salt_missing_label(true, false, true, None), not_enabled);
        assert_eq!(
            salt_missing_label(true, false, true, Some(false)),
            not_enabled
        );
        assert_eq!(
            salt_missing_label(true, false, true, Some(true)),
            "(no salt file yet; written on the first ping)"
        );
    }

    #[test]
    fn salt_missing_label_names_the_gate_that_actually_fired() {
        assert_eq!(
            salt_missing_label(false, true, false, Some(false)),
            "(not applicable; this build has no telemetry endpoint)"
        );
        assert_eq!(
            salt_missing_label(true, true, false, Some(false)),
            "(blocked by PRIVACYTK_TELEMETRY_DISABLED)"
        );
        assert_eq!(
            salt_missing_label(true, false, false, Some(true)),
            "(telemetry disabled in config.toml)"
        );
    }

    #[test]
    fn device_hash_line_renders_truncated_hash_when_salt_exists() {
        let expected = "  device hash:   01234567...89abcdef";
        assert_eq!(
            device_hash_line(true, false, true, Some(true), Some(SAMPLE_HASH)),
            expected
        );
    }
}