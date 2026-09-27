//! Heuristics for the `pkt hook cfd-context-injector` subcommand.
//!
//! Reads a Claude Code PreToolUse payload from stdin. When the Bash
//! command is architecturally significant (commit, branch, merge, package
//! init, ADR creation), extracts a topic token from the command, runs
//! `cfd <topic>`, and emits the ADR summary as a JSON
//! `hookSpecificOutput.additionalContext` block so the agent sees the
//! relevant decision before running the command.
//!
//! When the command is not architecturally significant (e.g.
//! `git status`, `cargo test`, `kubectl get`), the injector is a
//! silent no-op — no latency added to the workflow.

use serde_json::{json, Value};

use super::cfd::{
    locate_index, parse_index, read_adr, search, summarize_adr,
};

/// True if the given bash command is architecturally significant
/// (the agent is making a decision, not just inspecting state).
///
/// The check is structural: parse the first two tokens of the command
/// and see if the (binary, subcommand) pair looks like it changes
/// repo state. Observation verbs (`git status`, `cargo test`,
/// `kubectl get`) return false. Decision verbs (`git commit`,
/// `git checkout -b`, `cargo new`, `npm init`) return true.
pub fn is_architectural(command: &str) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() {
        return false;
    }
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    let mut i = 0;
    // Skip env=val prefixes.
    while i < tokens.len() && tokens[i].contains('=') && !tokens[i].starts_with('-') {
        i += 1;
    }
    let binary = tokens.get(i).copied().unwrap_or("");
    let subcommand = tokens.get(i + 1).copied().unwrap_or("");

    match (binary, subcommand) {
        // git: observation verbs are non-architectural.
        ("git", "status" | "log" | "diff" | "show" | "branch" | "remote" | "tag") => false,
        ("git", _) => true,
        ("gh", "pr" | "issue" | "repo") => true,
        ("gh", _) => false,
        // cargo
        ("cargo", "test" | "build" | "check" | "clippy" | "fmt" | "run") => false,
        ("cargo", _) => true,
        // package managers
        ("npm", "install" | "i" | "ci" | "test" | "run" | "ls" | "list") => false,
        ("npm", _) => true,
        ("pnpm" | "yarn", _) if subcommand != "test" && subcommand != "run" => true,
        ("pnpm" | "yarn", _) => false,
        // python
        ("pip" | "pip3" | "uv" | "poetry", "install" | "i") => true,
        ("pip" | "pip3" | "uv" | "poetry", _) => false,
        // k8s / containers / infra: state-changing only.
        ("kubectl", "apply" | "create" | "delete" | "patch" | "edit" | "scale" | "rollout") => true,
        ("kubectl", _) => false,
        ("docker", "build" | "run" | "create" | "rm" | "push" | "pull" | "tag") => true,
        ("docker", _) => false,
        ("helm", "install" | "upgrade" | "uninstall" | "rollback") => true,
        ("helm", _) => false,
        ("terraform" | "tofu", "apply" | "plan" | "destroy") => true,
        ("terraform" | "tofu", _) => false,
        // file ops: mkdir/touch always signal "I'm creating something".
        ("mkdir" | "touch" | "rm" | "mv" | "cp", _) => true,
        // generic observation fallthrough
        ("ls" | "cat" | "head" | "tail" | "find" | "grep" | "echo" | "pwd" | "wc", _) => false,
        // unknown binary: assume observation (safer; the LLM won't
        // get spammed with ADR context on commands we don't understand).
        _ => false,
    }
}

/// Extract a topic token from a Bash command. Looks for:
/// 1. Branch names after `git checkout -b <topic>` or `git switch -c`.
/// 2. Commit subjects after `git commit -m "<topic>"`.
/// 3. Path components after `mkdir <topic>`, `touch <topic>`, etc.
/// 4. First non-flag argument after the binary.
/// Returns None if no reasonable topic found.
pub fn extract_topic(command: &str) -> Option<String> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let mut i = 0;
    // Skip env-var prefixes (FOO=bar git commit ...).
    while i < tokens.len() && tokens[i].contains('=') && !tokens[i].starts_with('-') {
        i += 1;
    }
    let binary = tokens.get(i).copied().unwrap_or("");
    let subcommand = tokens.get(i + 1).copied().unwrap_or("");

    match (binary, subcommand) {
        // git checkout -b <topic> / switch -c <topic>
        ("git" | "gh", "checkout" | "switch" | "switch-branch") => {
            for j in (i + 2)..tokens.len().min(i + 6) {
                if matches!(tokens.get(j), Some(s) if *s == "-b" || *s == "-c" || *s == "--create") {
                    if let Some(topic) = tokens.get(j + 1) {
                        let cleaned = clean_topic_token(topic);
                        if !cleaned.is_empty() {
                            return Some(cleaned);
                        }
                    }
                }
            }
            None
        }
        // git commit -m "<topic>" / --message "<topic>"
        // Note: we join tokens after the flag because split_whitespace
        // does not respect shell quoting, so a single-quoted message
        // like 'add oauth login' would otherwise split into three tokens.
        ("git", "commit") => {
            for j in (i + 2)..tokens.len().min(i + 8) {
                if matches!(tokens.get(j), Some(s) if *s == "-m" || *s == "--message") {
                    // Join from j+1 to the next flag-like token or end.
                    let mut end = j + 1;
                    while end < tokens.len() && !tokens[end].starts_with('-') {
                        end += 1;
                    }
                    let joined = tokens[j + 1..end].join(" ");
                    let cleaned = clean_topic_token(&joined);
                    if !cleaned.is_empty() {
                        return Some(cleaned);
                    }
                }
            }
            if let Some(last) = tokens.last() {
                let cleaned = clean_topic_token(last);
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
            }
            None
        }
        // gh pr create --title "<topic>"
        ("gh", "pr") if tokens.get(i + 2) == Some(&"create") => {
            for j in (i + 3)..tokens.len() {
                if matches!(tokens.get(j), Some(s) if *s == "--title" || *s == "-t") {
                    // Join until next flag to handle quoted titles.
                    let mut end = j + 1;
                    while end < tokens.len() && !tokens[end].starts_with('-') {
                        end += 1;
                    }
                    let joined = tokens[j + 1..end].join(" ");
                    let cleaned = clean_topic_token(&joined);
                    if !cleaned.is_empty() {
                        return Some(cleaned);
                    }
                }
            }
            None
        }
        // mkdir/touch/rm/mv/cp <topic>: skip past flags
        ("mkdir" | "touch" | "rm" | "mv" | "cp", _) => {
            for j in (i + 1)..tokens.len() {
                let tok = tokens[j];
                if tok.starts_with('-') {
                    continue;
                }
                let cleaned = clean_topic_token(tok);
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
                break;
            }
            None
        }
        // cargo new <name>, npm init <name>, etc.
        ("cargo" | "npm" | "pnpm" | "yarn", "new" | "init" | "create") => {
            if let Some(name) = tokens.get(i + 2) {
                let cleaned = clean_topic_token(name);
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
            }
            None
        }
        // docker/kubectl/helm create <topic>
        ("docker" | "kubectl" | "helm", "create") => {
            if let Some(name) = tokens.get(i + 2) {
                let cleaned = clean_topic_token(name);
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
            }
            None
        }
        _ => None,
    }
}

fn clean_topic_token(s: &str) -> String {
    // Strip surrounding quotes, leading slashes, trailing slashes.
    let s = s.trim_matches(|c: char| c == '"' || c == '\'' || c == '/' || c == '`' || c == ',');
    // Replace separators with spaces, then take alphanumerics.
    s.replace(['-', '_', '.', '/'], " ")
        .split_whitespace()
        .filter(|t| t.chars().any(|c| c.is_alphanumeric()))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// Read the Claude Code PreToolUse payload from stdin and decide what
/// to inject. Returns the JSON the hook should print to stdout, or
/// an empty string if no injection is warranted.
pub fn run_hook(repo_root: &Path, max_bytes: usize) -> String {
    let mut input = String::new();
    if let Err(_) = std::io::stdin().read_to_string(&mut input) {
        return String::new();
    }
    let payload: Value = match serde_json::from_str(input.trim()) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    let command = payload
        .get("tool_input")
        .and_then(|ti| ti.get("command"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    if !is_architectural(command) {
        return String::new();
    }
    let Some(topic) = extract_topic(command) else {
        return String::new();
    };
    let idx_path = locate_index(repo_root);
    let Some(idx_path) = idx_path else {
        return String::new();
    };
    let Ok(idx) = parse_index(&idx_path) else {
        return String::new();
    };
    let hits = search(&idx, &topic);
    if hits.is_empty() {
        return String::new();
    }
    // Pick the top hit and emit a one-paragraph summary.
    let Some(adr) = hits.into_iter().next() else {
        return String::new();
    };
    let Some(body) = read_adr(&adr) else {
        return String::new();
    };
    let summary = summarize_adr(&body);
    let mut injection = format!(
        "[privacytk/cfd] Architectural decision relevant to `{}`:\n\n\
         ADR {} — {}\n\n\
         {}\n\n\
         (Full ADR at {}. Inject was auto-triggered because the command \
          looks architecturally significant.)",
        topic,
        adr.id,
        adr.title,
        summary,
        adr.path.display(),
    );
    if injection.len() > max_bytes {
        // Cap; truncate gracefully.
        injection.truncate(max_bytes.saturating_sub(60));
        injection.push_str("\n[...truncated; see ADR for full text...]\n");
    }
    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": injection
        }
    });
    output.to_string()
}

use std::io::Read;
use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_architectural_recognises_git_commit() {
        assert!(is_architectural("git commit -m 'add jwt validation'"));
        assert!(is_architectural("gh pr create --title 'jwt validation'"));
    }

    #[test]
    fn is_architectural_rejects_observation_verbs() {
        assert!(!is_architectural("git status"));
        assert!(!is_architectural("cargo test"));
        assert!(!is_architectural("kubectl get pods"));
        assert!(!is_architectural("ls -la"));
    }

    #[test]
    fn extract_topic_from_branch_name() {
        assert_eq!(
            extract_topic("git checkout -b feat/jwt-validation"),
            Some("feat jwt validation".to_string())
        );
        assert_eq!(
            extract_topic("git switch -c fix-bug"),
            Some("fix bug".to_string())
        );
    }

    #[test]
    fn extract_topic_from_commit_message() {
        // Whole commit message is the search query; the underlying
        // cfd::search will AND-match each whitespace-separated token.
        assert_eq!(
            extract_topic("git commit -m 'add oauth login'"),
            Some("add oauth login".to_string())
        );
    }

    #[test]
    fn extract_topic_from_mkdir() {
        assert_eq!(
            extract_topic("mkdir -p src/auth/handlers"),
            Some("src auth handlers".to_string())
        );
    }

    #[test]
    fn extract_topic_handles_env_var_prefix() {
        assert_eq!(
            extract_topic("FOO=bar git commit -m 'fix race'"),
            Some("fix race".to_string())
        );
    }

    #[test]
    fn extract_topic_returns_none_for_bare_command() {
        assert_eq!(extract_topic("ls"), None);
        assert_eq!(extract_topic(""), None);
    }

    #[test]
    fn clean_topic_token_strips_separators() {
        assert_eq!(clean_topic_token("feat/jwt-validation"), "feat jwt validation");
        assert_eq!(clean_topic_token("--my-option"), "my option");
        assert_eq!(clean_topic_token("'add jwt validation'"), "add jwt validation");
        assert_eq!(clean_topic_token("./src/auth"), "src auth");
    }
}
