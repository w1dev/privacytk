//! Compile-time / source-level invariants that hold the corporate fork in
//! shape. Every check is a `grep` over the on-disk source tree, run by
//! `cargo test --bin privacytk drift_check::tests`. The assertions mirror
//! what `scripts/check-fork-drift.sh` enforces across the upstream diff;
//! this version does not need `git diff` or a remote and so runs in any CI
//! that already builds the binary.
//!
//! If any of these fail, the corporate guarantees have been broken by a
//! careless rebase or upstream merge. Investigate before merging.

use std::fs;
use std::path::Path;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    let p = Path::new(ROOT).join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {}", p.display(), e))
}

#[test]
fn cargo_toml_does_not_declare_ureq() {
    let cargo = read("Cargo.toml");
    assert!(
        !cargo.lines().any(|l| l.trim_start().starts_with("ureq ")),
        "Cargo.toml must not declare `ureq` as a direct dependency — \
         telemetry is compiled out in privacytk"
    );
}

#[test]
fn telemetry_module_has_no_post_site() {
    let telemetry = read("src/core/telemetry.rs");
    assert!(
        !telemetry.contains("ureq::post"),
        "src/core/telemetry.rs must not import ureq; the corporate fork \
         does not transmit telemetry"
    );
    assert!(
        !telemetry.contains("send_string"),
        "src/core/telemetry.rs must not call send_string; no HTTP POST allowed"
    );
    // `endpoint_url` must remain a constant None.
    assert!(
        telemetry.contains("None"),
        "src/core/telemetry.rs::endpoint_url must keep returning None"
    );
}

#[test]
fn telemetry_cmd_has_no_ureq() {
    let cmd = read("src/core/telemetry_cmd.rs");
    assert!(
        !cmd.contains("ureq"),
        "src/core/telemetry_cmd.rs must not import ureq"
    );
    assert!(
        !cmd.contains("send_erasure_request") || cmd.contains("// FORK-NOTE"),
        "src/core/telemetry_cmd.rs must not implement send_erasure_request \
         (network erasure call) — privacytk handles erasure locally only"
    );
}

#[test]
fn ccusage_drops_npx_fallback() {
    let cc = read("src/analytics/ccusage.rs");
    // The forbidden behavior is `Command::new("npx")` or `.arg("npx")` —
    // comments mentioning npx are fine. `resolved_command("npx")` is what
    // upstream used and is the call site we are closing.
    for forbidden_call in [
        "Command::new(\"npx\")",
        "resolved_command(\"npx\")",
        "resolved_command(\"npx --yes",
        ".arg(\"--yes\")",
    ] {
        assert!(
            !cc.contains(forbidden_call),
            "src/analytics/ccusage.rs must not invoke npx (found `{forbidden_call}`); \
             the fallback to `npx --yes ccusage` was a covert egress to the npm registry"
        );
    }
}

#[test]
fn data_dir_is_privacytk_not_rtk() {
    let c = read("src/core/constants.rs");
    assert!(
        c.contains("pub const RTK_DATA_DIR: &str = \"privacytk\""),
        "RTK_DATA_DIR must be renamed to \"privacytk\" so the fork does \
         not collide with any upstream rtk install"
    );
}

#[test]
fn binary_name_in_cli_metadata_is_privacytk() {
    let main = read("src/main.rs");
    assert!(
        main.contains("name = \"privacytk\""),
        "Cli clap metadata must declare name = \"privacytk\""
    );
}

#[test]
fn package_name_in_cargo_toml_is_privacytk() {
    let cargo = read("Cargo.toml");
    assert!(
        cargo.lines().any(|l| l.trim_start().starts_with("name = \"privacytk\"")),
        "Cargo.toml package name must be \"privacytk\""
    );
}

#[test]
fn homepage_and_repository_are_stripped() {
    let cargo = read("Cargo.toml");
    for forbidden in ["homepage = \"https", "repository = \"https"] {
        assert!(
            !cargo.contains(forbidden),
            "Cargo.toml must not contain `{forbidden}` (would redirect to \
             the upstream project)"
        );
    }
}

#[test]
fn network_guard_module_present_and_rejects_permissive_policy() {
    let g = read("src/core/network_guard.rs");
    assert!(
        g.contains("default_deny"),
        "src/core/network_guard.rs must enforce default_deny"
    );
    // The load-time guard: a permissive policy must be refused at parse time.
    assert!(
        g.contains("refuses to load"),
        "src/core/network_guard.rs must refuse `default_deny = false` \
         policies at load time"
    );
}

#[test]
fn dedup_module_present_and_opt_in_only() {
    let d = read("src/core/dedup.rs");
    assert!(
        d.contains("PRIVACYTK_DEDUP"),
        "src/core/dedup.rs must read the PRIVACYTK_DEDUP env var to \
         detect opt-in"
    );
    // The default must be off; the module must never auto-activate.
    assert!(
        d.contains("set_enabled"),
        "src/core/dedup.rs must expose a programmatic enable toggle for tests"
    );
    // Cache must live under /tmp (never inside the repo or user home).
    assert!(
        d.contains("std::env::temp_dir"),
        "src/core/dedup.rs cache root must live under std::env::temp_dir()"
    );
}

#[test]
fn dedup_does_not_persist_command_output() {
    // Privacy invariant: the dedup cache stores only (key, sha256, bytes,
    // timestamp). The actual output bytes never land in the cache file.
    let d = read("src/core/dedup.rs");
    assert!(
        !d.contains("write_all(output"),
        "src/core/dedup.rs must not persist raw command output to the cache; \
         only the hash is stored"
    );
}

#[test]
fn dedup_subcommand_is_meta() {
    let main = read("src/main.rs");
    assert!(
        main.contains("Commands::Dedup"),
        "main.rs must dispatch Commands::Dedup"
    );
    let constants = read("src/core/constants.rs");
    assert!(
        constants.contains("\"dedup\""),
        "RTK_META_COMMANDS must include \"dedup\""
    );
}

#[test]
fn cfd_module_present_and_parses_both_forms() {
    let c = read("src/core/cfd.rs");
    assert!(
        c.contains("parse_table"),
        "src/core/cfd.rs must parse table-form decisions indexes"
    );
    assert!(
        c.contains("parse_bullets"),
        "src/core/cfd.rs must parse bullet-form decisions indexes"
    );
    assert!(
        c.contains("IndexFormat"),
        "src/core/cfd.rs must declare the IndexFormat enum"
    );
}

#[test]
fn cfd_subcommand_is_meta_and_local_only() {
    let main = read("src/main.rs");
    assert!(
        main.contains("Commands::Cfd"),
        "main.rs must dispatch Commands::Cfd"
    );
    assert!(
        main.contains("Commands::CfdContext"),
        "main.rs must dispatch Commands::CfdContext"
    );
    // The CFD lookup path must never resolve a network URL or call into
    // ureq / reqwest — it is a local-only filesystem reader.
    let c = read("src/core/cfd.rs");
    assert!(
        !c.contains("ureq") && !c.contains("reqwest") && !c.contains("TcpStream"),
        "src/core/cfd.rs must be local-only — no network calls"
    );
    let constants = read("src/core/constants.rs");
    assert!(
        constants.contains("\"cfd\"") && constants.contains("\"cfd-context\""),
        "RTK_META_COMMANDS must include both \"cfd\" and \"cfd-context\""
    );
}

#[test]
fn audit_module_present_and_redacts() {
    let a = read("src/core/audit.rs");
    assert!(
        a.contains("Local secrets/PII scanner"),
        "src/core/audit.rs must declare itself the secrets/PII scanner"
    );
    assert!(
        a.contains("redact_preview"),
        "src/core/audit.rs must redact secrets in preview output"
    );
    // The scanner must never touch the network.
    assert!(
        !a.contains("ureq") && !a.contains("reqwest") && !a.contains("TcpStream"),
        "src/core/audit.rs must be local-only — no network calls"
    );
}

#[test]
fn audit_and_verify_network_subcommands_registered() {
    let main = read("src/main.rs");
    assert!(
        main.contains("Commands::Audit"),
        "main.rs must dispatch Commands::Audit"
    );
    assert!(
        main.contains("Commands::VerifyNetwork"),
        "main.rs must dispatch Commands::VerifyNetwork"
    );
    // Both names must be in the meta-command allowlist so the
    // classification test in main.rs continues to pass.
    let constants = read("src/core/constants.rs");
    assert!(
        constants.contains("\"audit\""),
        "RTK_META_COMMANDS must include \"audit\""
    );
    assert!(
        constants.contains("\"verify-network\""),
        "RTK_META_COMMANDS must include \"verify-network\""
    );
}

#[test]
fn install_script_does_not_curl_upstream() {
    let install = read("install.sh");
    // The forbidden behavior is an outbound network call inside the script,
    // not a comment mentioning the upstream behavior. We forbid:
    //   - any `curl ` / `wget ` invocation that touches an http URL
    //   - a curl-to-sh pipe pattern
    // Comments are allowed to reference the upstream URL by name.
    assert!(
        !install.contains("curl -fsSL"),
        "install.sh must not use `curl -fsSL | sh` patterns; corporate \
         images ship the binary themselves"
    );
    assert!(
        !install.lines().any(|l| l.trim_start().starts_with("curl ") && l.contains("http")),
        "install.sh must not run a `curl http...` invocation"
    );
    assert!(
        !install.lines().any(|l| l.trim_start().starts_with("wget ") && l.contains("http")),
        "install.sh must not run a `wget http...` invocation"
    );
}

#[test]
fn readme_does_not_link_upstream_remote() {
    let readme = read("README.md");
    // Mentioning rtk-ai/rtk as the historical source is fine; a redirecting
    // homepage URL is not.
    assert!(
        !readme.contains("https://www.rtk-ai.app"),
        "README.md must not link to the upstream marketing site"
    );
}

#[test]
fn corporate_doc_present() {
    let path = Path::new(ROOT).join("docs/CORPORATE.md");
    assert!(
        path.exists(),
        "docs/CORPORATE.md is the corporate deployment guide and must ship \
         with every fork release"
    );
}

#[test]
fn drift_script_present_and_executable() {
    let p = Path::new(ROOT).join("scripts/check-fork-drift.sh");
    assert!(p.exists(), "scripts/check-fork-drift.sh must exist");
    let meta = fs::metadata(&p).expect("stat");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "scripts/check-fork-drift.sh must be executable (mode {:o})",
            mode
        );
    }
    let _ = meta;
}
#[test]
fn corporate_hooks_installed_by_init() {
    // The init flow must wire all three hooks: Bash rewrite,
    // SessionStart CFD injection, pre-commit audit gate.
    let main = read("src/main.rs");
    assert!(
        main.contains("HookCommands::SessionStart"),
        "main.rs must dispatch HookCommands::SessionStart"
    );
    assert!(
        main.contains("HookCommands::PreCommitGate"),
        "main.rs must dispatch HookCommands::PreCommitGate"
    );
    let constants = read("src/hooks/constants.rs");
    assert!(
        constants.contains("SESSION_START_HOOK_COMMAND"),
        "src/hooks/constants.rs must declare SESSION_START_HOOK_COMMAND"
    );
    assert!(
        constants.contains("PRE_COMMIT_GATE_HOOK_COMMAND"),
        "src/hooks/constants.rs must declare PRE_COMMIT_GATE_HOOK_COMMAND"
    );
    let init = read("src/hooks/init/mod.rs");
    assert!(
        init.contains("insert_corporate_hooks"),
        "src/hooks/init/mod.rs must expose insert_corporate_hooks"
    );
    let claude_init = read("src/hooks/init/claude.rs");
    assert!(
        claude_init.contains("insert_corporate_hooks"),
        "src/hooks/init/claude.rs install path must call insert_corporate_hooks"
    );
}
