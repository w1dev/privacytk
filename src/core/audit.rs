//! Local secrets/PII scanner used by `privacytk audit`.
//!
//! This module never opens a socket, never logs hits off-host, and never
//! caches matches between invocations. It exists so operators can scan a tree
//! they are about to feed into an LLM (or that an LLM might already be reading
//! from) and decide whether the contents are safe to ship.
//!
//! The patterns are deliberately conservative: high recall, low precision.
//! The output says what matched and where; it does not modify the input.

use anyhow::Result;
use regex::RegexSet;
use serde::Serialize;
use std::path::Path;
use walkdir::WalkDir;

/// Categories the scanner recognizes. Each maps to a small set of regexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    AwsAccessKey,
    AwsSecretKey,
    PrivateKeyBlock,
    GithubToken,
    SlackToken,
    StripeKey,
    GoogleApiKey,
    Jwt,
    EmailAddress,
    CreditCardPan,
    Iban,
    SsnUs,
    IpAddress,
    UrlWithCredentials,
}

impl Category {
    pub fn label(&self) -> &'static str {
        match self {
            Category::AwsAccessKey => "AWS access key id",
            Category::AwsSecretKey => "AWS secret access key",
            Category::PrivateKeyBlock => "PEM private key block",
            Category::GithubToken => "GitHub token (ghp_/gho_/ghs_/ghu_)",
            Category::SlackToken => "Slack token (xox[bpars]-)",
            Category::StripeKey => "Stripe live/test key (sk_live_/sk_test_/rk_live_)",
            Category::GoogleApiKey => "Google API key (AIza...)",
            Category::Jwt => "JSON Web Token (eyJ... header)",
            Category::EmailAddress => "Email address",
            Category::CreditCardPan => "Credit-card-like PAN (13–19 digits)",
            Category::Iban => "IBAN",
            Category::SsnUs => "US Social Security Number (NNN-NN-NNNN)",
            Category::IpAddress => "IPv4 address",
            Category::UrlWithCredentials => "URL with embedded credentials",
        }
    }
}

/// Pattern table. Order matters only for the array; the regex set is the
/// authoritative matcher.
const PATTERNS: &[(Category, &str)] = &[
    (Category::AwsAccessKey, r"\bAKIA[0-9A-Z]{16}\b"),
    (
        Category::AwsSecretKey,
        "(?i)aws(.{0,20})?(secret|access)?_?(key)?(.{0,20})?['\"][0-9a-zA-Z/+]{40}['\"]",
    ),
    (Category::PrivateKeyBlock, r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    (
        Category::GithubToken,
        r"\b(ghp|gho|ghs|ghu)_[A-Za-z0-9]{36,}\b",
    ),
    (
        Category::SlackToken,
        r"\bxox[bpars]-[A-Za-z0-9-]{10,}\b",
    ),
    (
        Category::StripeKey,
        r"\b(sk|rk)_(live|test)_[0-9a-zA-Z]{16,}\b",
    ),
    (Category::GoogleApiKey, r"\bAIza[0-9A-Za-z_-]{35}\b"),
    (Category::Jwt, r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b"),
    // Email: anchored with a simple character class. False positives on commit
    // hashes are accepted — operators can grep the output.
    (Category::EmailAddress, r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"),
    // PAN: 13–19 digits separated by optional spaces or dashes. Intentionally
    // permissive; the Luhn check is left to the operator/UI.
    (
        Category::CreditCardPan,
        r"\b(?:\d[ -]*?){13,19}\b",
    ),
    (
        Category::Iban,
        r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b",
    ),
    (
        Category::SsnUs,
        r"\b\d{3}-\d{2}-\d{4}\b",
    ),
    (Category::IpAddress, r"\b(?:\d{1,3}\.){3}\d{1,3}\b"),
    (
        Category::UrlWithCredentials,
        r"https?://[A-Za-z0-9._%+-]+:[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+",
    ),
];

/// One hit from the scanner. Coordinates are line numbers (1-based) and column
/// (0-based, byte offset within the line).
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub file: String,
    pub line: u64,
    pub column: u64,
    pub category: Category,
    /// Redacted preview: first 4 bytes of the match, then a marker, then the
    /// last 2 bytes (when the match is long enough). The full secret is never
    /// echoed.
    pub preview: String,
}

/// Build a redacted preview of the raw match. Avoids printing the secret
/// itself even when the operator reads the JSON output directly.
pub fn redact_preview(raw: &str) -> String {
    let len = raw.chars().count();
    if len <= 8 {
        return "***".into();
    }
    let head: String = raw.chars().take(4).collect();
    let tail: String = raw.chars().rev().take(2).collect::<String>().chars().rev().collect();
    format!("{head}…{tail} ({len} chars)")
}

pub struct Scanner {
    /// Compiled regexes, one per PATTERNS entry, used to recover match spans
    /// (start/end byte offsets) for redacted previews. The RegexSet is kept
    /// alongside as a fast pre-filter so we only call `Regex::find_iter` on
    /// lines that match at least one pattern.
    set: RegexSet,
    patterns: Vec<(Category, regex::Regex)>,
}

impl Scanner {
    pub fn new() -> Result<Self> {
        let pats: Vec<&'static str> = PATTERNS.iter().map(|(_, p)| *p).collect();
        let set = RegexSet::new(&pats)?;
        let patterns = PATTERNS
            .iter()
            .map(|(cat, pat)| Ok::<_, anyhow::Error>((*cat, regex::Regex::new(pat)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { set, patterns })
    }

    /// Scan a single file's contents. Returns every match; the caller filters
    /// by file extension or path if needed.
    pub fn scan_text(&self, path: &Path, content: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        let path_str = path.display().to_string();
        for (line_no, line) in content.lines().enumerate() {
            // Fast pre-filter: if the line matches no pattern at all, skip.
            if !self.set.is_match(line) {
                continue;
            }
            for (cat, pat) in &self.patterns {
                for m in pat.find_iter(line) {
                    hits.push(Hit {
                        file: path_str.clone(),
                        line: (line_no + 1) as u64,
                        column: m.start() as u64,
                        category: *cat,
                        preview: redact_preview(m.as_str()),
                    });
                }
            }
        }
        hits
    }

    /// Walk `root` and scan every regular file whose name does not start with
    /// `.git/`. Symlinks are not followed. Binary detection is heuristic:
    /// files containing a NUL byte in the first 8 KiB are skipped.
    pub fn scan_tree(&self, root: &Path) -> Vec<Hit> {
        let mut all = Vec::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // Skip .git entirely.
                e.file_name() != std::ffi::OsStr::new(".git")
            })
        {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            if bytes.contains(&0) {
                continue;
            }
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            all.extend(self.scan_text(path, text));
        }
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(label: &str) -> Category {
        match label {
            "AWS access key id" => Category::AwsAccessKey,
            "GitHub token (ghp_/gho_/ghs_/ghu_)" => Category::GithubToken,
            "PEM private key block" => Category::PrivateKeyBlock,
            "Email address" => Category::EmailAddress,
            "JSON Web Token (eyJ... header)" => Category::Jwt,
            other => panic!("unexpected label in test: {other}"),
        }
    }

    #[test]
    fn finds_aws_access_key() {
        let s = Scanner::new().unwrap();
        let hits = s.scan_text(
            std::path::Path::new("env"),
            "AWS_ACCESS_KEY_ID=AKIAABCDEFGHIJKLMNOP\n",
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].category, cat("AWS access key id"));
    }

    #[test]
    fn finds_github_token_and_redacts() {
        let s = Scanner::new().unwrap();
        let token = "ghp_".to_string() + &"a".repeat(40);
        let hits =
            s.scan_text(std::path::Path::new(".npmrc"), &format!("token = {token}\n"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].category, cat("GitHub token (ghp_/gho_/ghs_/ghu_)"));
        assert!(!hits[0].preview.contains(&token), "preview leaked the token");
        assert!(hits[0].preview.contains('…'));
    }

    #[test]
    fn finds_pem_block() {
        let s = Scanner::new().unwrap();
        let hits = s.scan_text(
            std::path::Path::new("k.pem"),
            "-----BEGIN RSA PRIVATE KEY-----\nstuff\n-----END RSA PRIVATE KEY-----\n",
        );
        assert_eq!(hits[0].category, cat("PEM private key block"));
    }

    #[test]
    fn finds_jwt() {
        let s = Scanner::new().unwrap();
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyIn0.signature_signature_signature";
        let hits = s.scan_text(std::path::Path::new("auth.txt"), jwt);
        assert!(hits.iter().any(|h| h.category == cat("JSON Web Token (eyJ... header)")));
    }

    #[test]
    fn finds_email() {
        let s = Scanner::new().unwrap();
        let hits = s.scan_text(
            std::path::Path::new("contact.txt"),
            "reach me at alice@example.com\n",
        );
        assert!(hits.iter().any(|h| h.category == cat("Email address")));
    }

    #[test]
    fn redact_short_strings() {
        assert_eq!(redact_preview("abc"), "***");
        assert_eq!(redact_preview(""), "***");
        assert!(redact_preview("longersecret").contains('…'));
    }
}