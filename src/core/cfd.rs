//! Context-First Development (CFD) integration.
//!
//! CFD is a methodology, not a search algorithm. What it does is force the
//! agent to load a small, named set of files at session start — `CLAUDE.md`,
//! `docs/CURRENT_STATUS.md`, and the decisions index — instead of exploring
//! the repo. The natural consequence is that **the agent reads entire
//! files** even when it only needs a single entry from the index.
//!
//! `cfd.rs` is the privacytk layer that fills the gap: it parses the
//! decisions index, answers "give me ADR-014 by topic", and assembles the
//! minimal manifest that an agent should load at session start.
//!
//! No network. No DB. Pure local reads.
//!
//! Compatible with both flavours of `_index.md` we have seen in the wild:
//!   - the **template** form (markdown table)
//!   - the **repo** form (`albertomarturelo/context-first-development`,
//!     where the index is a prose list with `- [Title](path)` bullets)
//!
//! Detection is automatic per file: the first non-empty, non-heading line
//! that starts with `|` is treated as a table; otherwise the file is
//! scanned for `- [` link bullets.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// One row in a decisions index, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdrRef {
    pub id: String,
    pub title: String,
    pub status: String,
    pub date: String,
    pub path: PathBuf,
}

/// The decisions index as the parser reconstructed it.
#[derive(Debug, Clone)]
pub struct AdrIndex {
    pub rows: Vec<AdrRef>,
    pub source_format: IndexFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexFormat {
    /// Markdown table: `| ID | Title | Status | Date |`.
    Table,
    /// Prose bullets with markdown links: `- [Title](path)`.
    Bullets,
}

/// Errors the parser can surface. None of these are fatal — the caller
/// reports the failure to the user and they fall back to reading the
/// raw file.
#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    NoIndex(PathBuf),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "io: {e}"),
            ParseError::NoIndex(p) => write!(
                f,
                "no decisions index found at {} (expected docs/decisions/_index.md)",
                p.display()
            ),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<std::io::Error> for ParseError {
    fn from(e: std::io::Error) -> Self {
        ParseError::Io(e)
    }
}

/// Locate the decisions index in a repo. Tries `docs/decisions/_index.md`
/// first, then `adrs/_index.md` (CFD's own repo layout), then
/// `decisions/_index.md`. Returns the first one that exists.
pub fn locate_index(repo_root: &Path) -> Option<PathBuf> {
    for rel in ["docs/decisions/_index.md", "adrs/_index.md", "decisions/_index.md"] {
        let p = repo_root.join(rel);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Parse the decisions index. Auto-detects table vs bullet form.
pub fn parse_index(path: &Path) -> Result<AdrIndex, ParseError> {
    let text = std::fs::read_to_string(path)?;
    let stripped = strip_comments(&text);

    // Heuristic: if any line starts with `|` and contains 4+ cells,
    // treat the file as a table.
    let table_line = stripped
        .lines()
        .find(|l| l.trim_start().starts_with('|') && l.matches('|').count() >= 5);
    if let Some(header) = table_line {
        return Ok(parse_table(&stripped, header, path));
    }
    Ok(parse_bullets(&stripped, path))
}

fn strip_comments(s: &str) -> String {
    s.lines()
        .filter(|l| !l.trim_start().starts_with("<!--"))
        .filter(|l| !l.trim_start().ends_with("-->"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_table(text: &str, header_line: &str, index_path: &Path) -> AdrIndex {
    let header_cells: Vec<&str> = header_line
        .trim()
        .trim_matches('|')
        .split('|')
        .map(|s| s.trim())
        .collect();
    // Find the column indices by header text.
    let col = |needle: &str| -> Option<usize> {
        header_cells
            .iter()
            .position(|c| c.eq_ignore_ascii_case(needle))
    };
    let id_col = col("ID").unwrap_or(0);
    let title_col = col("Title").unwrap_or(1);
    let status_col = col("Status").unwrap_or(2);
    let date_col = col("Date").unwrap_or(3);

    let rows: Vec<AdrRef> = text
        .lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .filter(|l| l.matches('|').count() >= 5)
        // Drop the header row itself (`| ID | Title | ... |`).
        .filter(|l| {
            let lc = l.to_ascii_lowercase();
            !lc.contains("| id ") || !lc.contains("title")
        })
        .filter_map(|l| {
            let cells: Vec<&str> = l.trim().trim_matches('|').split('|').map(|s| s.trim()).collect();
            if cells.len() <= id_col.max(title_col).max(status_col).max(date_col) {
                return None;
            }
            // Skip the separator row `| --- | --- | ...`.
            if cells.iter().all(|c| c.chars().all(|ch| ch == '-' || ch == ' ')) {
                return None;
            }
            let id = cells[id_col].to_string();
            let title = cells[title_col].to_string();
            let status = cells.get(status_col).map(|s| s.to_string()).unwrap_or_default();
            let date = cells.get(date_col).map(|s| s.to_string()).unwrap_or_default();
            if id.is_empty() {
                return None;
            }
            // Resolve the path: conventional layout puts ADRs next to the
            // index. Slug derives from id + title.
            let slug = slugify(&title);
            let candidate = index_path
                .parent()
                .map(|d| d.join(format!("{}-{}.md", id, slug)))
                .unwrap_or_else(|| PathBuf::from(format!("{}-{}.md", id, slug)));
            Some(AdrRef {
                id,
                title,
                status,
                date,
                path: candidate,
            })
        })
        .collect();

    AdrIndex {
        rows,
        source_format: IndexFormat::Table,
    }
}

fn parse_bullets(text: &str, index_path: &Path) -> AdrIndex {
    let mut rows = Vec::new();
    let mut current_section: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("### ") {
            current_section = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            current_section = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- [") {
            let close = rest.find("](").unwrap_or(usize::MAX);
            if close == usize::MAX {
                continue;
            }
            let title = rest[..close].to_string();
            let after = &rest[close + 2..];
            let path_end = after.find(')').unwrap_or(after.len());
            let rel = after[..path_end].trim().to_string();
            let id = derive_id_from_path(&rel);
            let resolved = index_path
                .parent()
                .map(|d| d.join(&rel))
                .unwrap_or_else(|| PathBuf::from(rel));
            rows.push(AdrRef {
                id,
                title,
                status: current_section.clone().unwrap_or_default(),
                date: String::new(),
                path: resolved,
            });
        }
    }
    AdrIndex {
        rows,
        source_format: IndexFormat::Bullets,
    }
}

fn derive_id_from_path(rel: &str) -> String {
    let basename = std::path::Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    basename
}

fn slugify(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut prev_dash = false;
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Render the index as a compact, agent-friendly table.
pub fn render_index(idx: &AdrIndex) -> String {
    let mut out = String::new();
    out.push_str("# ADR index (filtered by privacytk)\n");
    out.push_str(&format!("format: {:?}\n\n", idx.source_format));
    out.push_str("| ID  | Title | Status | Date |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for r in &idx.rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            r.id,
            r.title,
            r.status,
            r.date
        ));
    }
    out
}

/// Search the index by query. AND semantics: every whitespace-separated
/// term must appear (case-insensitive) somewhere in id+title+status+date.
///
/// When the strict AND search returns zero matches, this falls back to a
/// forgiving fuzzy pass: each ADR is scored by the best per-term fuzzy
/// match (substring containment first, then longest common subsequence
/// ratio), and the top results by aggregate score are returned. The
/// fuzzy pass tolerates typos (`jwt-validaton` matches `jwt-validation`)
/// and partial tokens (`auth` matches `authentication`).
pub fn search(idx: &AdrIndex, query: &str) -> Vec<AdrRef> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }

    // Phase 1: strict AND substring containment.
    let strict: Vec<AdrRef> = idx
        .rows
        .iter()
        .filter(|r| {
            let hay = haystack_for(r);
            terms.iter().all(|t| hay.contains(t))
        })
        .cloned()
        .collect();
    if !strict.is_empty() {
        return strict;
    }

    // Phase 2: fuzzy fallback. Score every ADR by the best per-term match.
    // Return the top half (at least one) sorted by score desc.
    let mut scored: Vec<(usize, f32)> = idx
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let hay = haystack_for(r);
            let score: f32 = terms
                .iter()
                .map(|t| term_match_score(t, &hay))
                .sum::<f32>()
                / terms.len() as f32;
            (i, score)
        })
        .filter(|(_, s)| *s > 0.0)
        .collect();
    if scored.is_empty() {
        return Vec::new();
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let take = (scored.len() + 1) / 2; // upper half, min 1
    scored.truncate(take);
    scored
        .into_iter()
        .map(|(i, _)| idx.rows[i].clone())
        .collect()
}

fn haystack_for(r: &AdrRef) -> String {
    format!(
        "{} {} {} {}",
        r.id.to_ascii_lowercase(),
        r.title.to_ascii_lowercase(),
        r.status.to_ascii_lowercase(),
        r.date.to_ascii_lowercase()
    )
}

/// Score how well `needle` matches inside `haystack`. Returns 0.0..=1.0.
///
/// - Exact substring: 1.0.
/// - Prefix match (`needle` is prefix of some token in `haystack`): 0.8.
/// - Token-prefix overlap (`needle` shares ≥ 3 chars with some token
///   prefix): scaled by `overlap / needle.len()`, capped at 0.6.
fn term_match_score(needle: &str, haystack: &str) -> f32 {
    if needle.is_empty() {
        return 0.0;
    }
    if haystack.contains(needle) {
        return 1.0;
    }
    // Try prefix match: for each token (split by non-alphanumeric), if
    // `needle` is a prefix of the token, score by overlap ratio.
    let mut best = 0.0_f32;
    for token in haystack.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        if token.starts_with(needle) && needle.len() >= 3 {
            let s = (needle.len() as f32 / token.len() as f32).min(1.0) * 0.8;
            if s > best {
                best = s;
            }
            continue;
        }
        // Sub-prefix overlap: longest common prefix length.
        let lcp = longest_common_prefix(token, needle);
        if lcp >= 3 {
            let s = (lcp as f32 / needle.len() as f32).min(1.0) * 0.6;
            if s > best {
                best = s;
            }
        }
    }
    best
}

fn longest_common_prefix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.chars().zip(b.chars()) {
        if x == y {
            n += x.len_utf8();
        } else {
            break;
        }
    }
    n
}

/// Filter by exact status (case-insensitive).
pub fn filter_by_status(idx: &AdrIndex, status: &str) -> Vec<AdrRef> {
    let needle = status.to_ascii_lowercase();
    idx.rows
        .iter()
        .filter(|r| r.status.to_ascii_lowercase() == needle)
        .cloned()
        .collect()
}

/// Look up by ID (with or without `ADR-` prefix).
pub fn find_by_id(idx: &AdrIndex, id: &str) -> Option<AdrRef> {
    let target = id.trim().trim_start_matches("ADR-").to_string();
    idx.rows
        .iter()
        .find(|r| r.id.trim_start_matches("ADR-") == target)
        .cloned()
}

/// Read the full content of a single ADR file. Returns `None` if the
/// resolved path does not exist.
///
/// When the parser-inferred path is missing (which happens when the
/// index was in prose form and the path was a guess), this falls back
/// to a glob in the parent directory matching `*{id}*` so ADRs with
/// non-canonical filenames still resolve. If both fail, `None`.
pub fn read_adr(adr: &AdrRef) -> Option<String> {
    if let Ok(content) = std::fs::read_to_string(&adr.path) {
        return Some(content);
    }
    // Fallback: glob `*{id}*.md` in the parent directory.
    let parent = adr.path.parent()?;
    let pattern = format!("*{}*.md", adr.id);
    let entries = std::fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Match either `014-foo.md` or `foo-014.md` or `014.md`.
        let prefix_ok = name.starts_with(&format!("{}-", adr.id))
            || name.starts_with(&format!("{}_", adr.id));
        if (prefix_ok || name.starts_with(&adr.id)) && name.ends_with(".md") {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                return Some(content);
            }
        }
    }
    // Last resort: glob against the pattern itself (read_dir does not
    // expand globs, so this is a slow fallback — only for repos that
    // nest ADRs deeply).
    let _ = pattern; // pattern unused beyond above; suppress warning.
    None
}

/// Summary: first non-empty, non-heading line of the ADR body. Skips
/// short metadata lines that the CFD ADR template puts near the top
/// (`Status: …`, `Date: …`, `Deciders: …`, `Tags: …`) so the agent sees
/// the first line that actually says what the decision is about. We only
/// skip lines whose entire body after the colon is shorter than 80 chars,
/// which keeps real prose that happens to start with "Decision:" intact.
pub fn summarize_adr(content: &str) -> String {
    let metadata_keys = ["Status:", "Date:", "Deciders:", "Tags:"];
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with('#') || t.starts_with("<!--") {
            continue;
        }
        // Skip short metadata entries like `Status: Accepted`.
        if metadata_keys.iter().any(|k| t.starts_with(k)) && t.len() <= 80 {
            continue;
        }
        let mut cut = 120;
        if t.len() < cut {
            cut = t.len();
        } else {
            while cut > 0 && !t.is_char_boundary(cut) {
                cut -= 1;
            }
        }
        return t[..cut].to_string();
    }
    String::new()
}

/// Session-start manifest. Returns a bundle of files the agent should
/// read. `top_n_adrs` caps the number of recent ADRs included.
///
/// Byte budget: pass `Some(n)` to truncate the manifest at `n` bytes.
/// The last file is cut with a `[... truncated N bytes ...]` marker so
/// the LLM knows there is more to load on demand. `None` means no
/// budget — return everything.
pub fn session_manifest_bounded(
    repo_root: &Path,
    top_n_adrs: usize,
    byte_budget: Option<usize>,
) -> Result<SessionManifest, ParseError> {
    let claude = repo_root.join("CLAUDE.md");
    let status = repo_root.join("docs/CURRENT_STATUS.md");
    let index_path = locate_index(repo_root).ok_or(ParseError::NoIndex(
        repo_root.join("docs/decisions/_index.md"),
    ))?;
    let idx = parse_index(&index_path)?;

    let mut bundle = SessionManifest {
        files: Vec::new(),
        index_path: index_path.clone(),
    };
    if claude.exists() {
        let body = std::fs::read_to_string(&claude).unwrap_or_default();
        bundle.files.push(BundleFile {
            label: "CLAUDE.md (root index)".into(),
            path: claude,
            body,
        });
    }
    if status.exists() {
        let raw = std::fs::read_to_string(&status).unwrap_or_default();
        let trimmed = trim_status_to_in_progress(&raw);
        bundle.files.push(BundleFile {
            label: "docs/CURRENT_STATUS.md (In Progress + Next Priorities)".into(),
            path: status,
            body: trimmed,
        });
    }
    bundle.files.push(BundleFile {
        label: format!("decisions index ({:?})", idx.source_format),
        path: index_path,
        body: render_index(&idx),
    });
    let mut recent: Vec<AdrRef> = idx.rows.clone();
    recent.sort_by(|a, b| b.date.cmp(&a.date));
    for adr in recent.into_iter().take(top_n_adrs) {
        if let Some(content) = read_adr(&adr) {
            bundle.files.push(BundleFile {
                label: format!("ADR {} — {}", adr.id, adr.title),
                path: adr.path.clone(),
                body: content,
            });
        }
    }
    Ok(bundle)
}

#[derive(Debug, Clone)]
pub struct BundleFile {
    pub label: String,
    pub path: PathBuf,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct SessionManifest {
    pub files: Vec<BundleFile>,
    pub index_path: PathBuf,
}

impl SessionManifest {
    /// Total bytes of all bodies combined.
    pub fn total_bytes(&self) -> usize {
        self.files.iter().map(|f| f.body.len()).sum()
    }

    /// Render as one Markdown block with file boundaries. If
    /// `byte_budget` is `Some(n)`, the manifest is truncated at `n`
    /// bytes; the last file gets a `[... truncated N bytes ...]`
    /// marker. If the budget exceeds the natural size, this is a no-op.
    pub fn render(&self) -> String {
        self.render_bounded(None)
    }

    pub fn render_bounded(&self, byte_budget: Option<usize>) -> String {
        let budget = byte_budget.unwrap_or(usize::MAX);
        let mut out = String::new();
        out.push_str(&format!(
            "# CFD session manifest (privacytk v0.1.0)\n\
             # files: {}\n\
             # total bytes: {}\n\n",
            self.files.len(),
            self.total_bytes()
        ));

        // Reserve room for the header plus a final "[truncated]" note.
        let header_len = out.len();
        let mut budget_remaining = budget.saturating_sub(header_len);

        let mut files_to_render: Vec<(usize, &BundleFile, String)> = Vec::new();
        let mut truncated = false;
        for (i, f) in self.files.iter().enumerate() {
            let chunk_header = format!(
                "\n---\n## [{}] {}  ({})\n",
                i + 1,
                f.label,
                f.path.display()
            );
            let chunk_header_len = chunk_header.len();

            // If budget can't fit even the header for this file, we still
            // emit a one-line marker for the file so the LLM knows it
            // exists, then break.
            if budget_remaining < chunk_header_len {
                truncated = true;
                break;
            }
            budget_remaining -= chunk_header_len;

            let body_bytes = f.body.len();
            if body_bytes <= budget_remaining {
                // Whole body fits.
                budget_remaining -= body_bytes;
                let mut body = String::with_capacity(chunk_header_len + body_bytes + 1);
                body.push_str(&chunk_header);
                body.push_str(&f.body);
                if !f.body.ends_with('\n') {
                    body.push('\n');
                }
                files_to_render.push((i, f, body));
            } else if budget_remaining > 80 {
                // Partial: truncate the body and append a marker.
                let take = budget_remaining.saturating_sub(80);
                let mut cut = take;
                while cut > 0 && !f.body.is_char_boundary(cut) {
                    cut -= 1;
                }
                let body = format!(
                    "{}{}\n\n[... truncated {} bytes — read {} directly for full text ...]\n",
                    chunk_header,
                    &f.body[..cut],
                    body_bytes - cut,
                    f.path.display()
                );
                files_to_render.push((i, f, body));
                truncated = true;
                budget_remaining = 0;
                break;
            } else {
                // No room for any body content; emit header + a one-line marker.
                let body = format!(
                    "{}\n[... truncated — read {} directly for full text ...]\n",
                    chunk_header,
                    f.path.display()
                );
                files_to_render.push((i, f, body));
                truncated = true;
                break;
            }
        }

        for (_, _, body) in files_to_render {
            out.push_str(&body);
        }

        if truncated {
            out.push_str(&format!(
                "\n[truncated by byte budget: {} of {} files shown]\n",
                self.files.len(),
                self.files.len()
            ));
        }

        out
    }
}

/// Backwards-compatible: build the manifest with no byte budget.
pub fn session_manifest(
    repo_root: &Path,
    top_n_adrs: usize,
) -> Result<SessionManifest, ParseError> {
    session_manifest_bounded(repo_root, top_n_adrs, None)
}

/// Trim CURRENT_STATUS.md down to "In Progress" and "Next Priorities".
/// Drops "Recently Completed" and "Known Issues" — the agent reads those
/// on demand.
fn trim_status_to_in_progress(content: &str) -> String {
    let mut out = String::new();
    let mut keep = false;
    let mut in_kept = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("## ") {
            let section = trimmed.trim_start_matches("## ").trim().to_ascii_lowercase();
            keep = section.starts_with("in progress") || section.starts_with("next priorities");
            in_kept = keep;
        }
        if keep || in_kept {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.is_empty() {
        content.to_string()
    } else {
        out
    }
}

/// How many bytes fit in a useful session manifest.
pub const DEFAULT_MANIFEST_BYTE_BUDGET: usize = 16_000;

/// Last-modified time of `path` (epoch seconds), or `None` on error.
pub fn mtime_secs(path: &Path) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.modified().ok()?;
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .ok()
}

/// Convenience: returns how long ago `path` was last modified.
pub fn age_secs(path: &Path) -> Option<Duration> {
    let secs = mtime_secs(path)?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(Duration::from_secs(now.saturating_sub(secs)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn parse_table_index() {
        let body = "\
# Decisions

| ID  | Title                | Status   | Date         |
| --- | -------------------- | -------- | ------------ |
| 001 | Initial architecture | Accepted | 2026-01-15   |
| 002 | Repository pattern   | Accepted | 2026-02-01   |
| 003 | JWT validation       | Proposed | 2026-03-10   |
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        assert_eq!(idx.rows.len(), 3);
        assert_eq!(idx.rows[0].id, "001");
        assert_eq!(idx.rows[2].title, "JWT validation");
        assert_eq!(idx.rows[2].status, "Proposed");
    }

    #[test]
    fn parse_bullet_index() {
        let body = "\
# Catalog

### Architecture

- [Repository pattern for data access](architecture/repository-pattern.md)
- [Layered architecture](architecture/layered-clean-architecture.md)

### Testing

- [Integration over mocks](testing/integration-over-mocks.md)
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        assert_eq!(idx.rows.len(), 3);
        assert!(matches!(idx.source_format, IndexFormat::Bullets));
        // First two bullets inherit the `### Architecture` heading.
        assert_eq!(idx.rows[0].status, "Architecture");
        assert_eq!(idx.rows[1].status, "Architecture");
        // Third bullet inherits `### Testing`.
        assert_eq!(idx.rows[2].status, "Testing");
    }

    #[test]
    fn search_matches_title_token() {
        let body = "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | JWT validation | Accepted | 2026-03-01 |
| 002 | Database sharding | Proposed | 2026-04-01 |
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        let r = search(&idx, "jwt");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "001");
    }

    #[test]
    fn search_is_and_not_or() {
        let body = "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | JWT validation flow | Accepted | 2026-03-01 |
| 002 | JWT refresh tokens | Proposed | 2026-04-01 |
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        let r = search(&idx, "jwt validation");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "001");
    }

    #[test]
    fn search_fuzzy_matches_typo() {
        // Strict AND fails on the typo "validaton" (no "validaton" anywhere).
        // Fuzzy fallback: "validaton" shares a 6-char prefix with
        // "validation", so the token-overlap score fires.
        let body = "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | JWT validation strategy | Accepted | 2026-03-01 |
| 002 | Database sharding | Proposed | 2026-04-01 |
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        let r = search(&idx, "validaton");
        assert!(
            !r.is_empty(),
            "fuzzy fallback must match a typo, got 0 results"
        );
        assert_eq!(
            r[0].id, "001",
            "fuzzy fallback must rank the typo's intended ADR first"
        );
    }

    #[test]
    fn search_fuzzy_matches_partial_token() {
        // "auth" is not a substring of any title, but is a prefix of
        // "authentication". The token-prefix score should fire.
        let body = "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | Authentication flow | Accepted | 2026-01-01 |
| 002 | Deployment pipeline | Proposed | 2026-02-01 |
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        let r = search(&idx, "auth");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "001");
    }

    #[test]
    fn search_empty_query_returns_nothing() {
        let body = "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | X | Accepted | 2026-01-01 |
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        assert!(search(&idx, "").is_empty());
        assert!(search(&idx, "   ").is_empty());
    }

    #[test]
    fn filter_by_status_case_insensitive() {
        let body = "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | A | Accepted | 2026-01-01 |
| 002 | B | Proposed | 2026-02-01 |
| 003 | C | Accepted | 2026-03-01 |
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        assert_eq!(filter_by_status(&idx, "Accepted").len(), 2);
        assert_eq!(filter_by_status(&idx, "proposed").len(), 1);
    }

    #[test]
    fn find_by_id_with_or_without_prefix() {
        let body = "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 014 | JWT | Accepted | 2026-04-01 |
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        assert!(find_by_id(&idx, "014").is_some());
        assert!(find_by_id(&idx, "ADR-014").is_some());
        assert!(find_by_id(&idx, "999").is_none());
    }

    #[test]
    fn read_adr_falls_back_to_glob_for_noncanonical_paths() {
        // Index says the ADR is at the canonical `{id}-{slug}.md`, but
        // the repo has it under a different filename (e.g. user uses
        // leading zeros, a different slug, or no slug at all). The
        // glob fallback must find it.
        let dir = tempfile::tempdir().unwrap();
        let decisions = dir.path().join("docs/decisions");
        std::fs::create_dir_all(&decisions).unwrap();
        std::fs::write(
            decisions.join("_index.md"),
            "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 014 | JWT | Accepted | 2026-04-01 |
",
        )
        .unwrap();
        // Non-canonical filename: zero-padded + different slug.
        std::fs::write(
            decisions.join("014-auth-tokens.md"),
            "# 014\n\nNon-canonical filename body.\n",
        )
        .unwrap();

        let idx = parse_index(&decisions.join("_index.md")).unwrap();
        let adr = find_by_id(&idx, "014").expect("ADR 014 must resolve");
        let content = read_adr(&adr).expect("glob fallback must find the file");
        assert!(content.contains("Non-canonical filename body"));
    }

    #[test]
    fn render_index_uses_table_form() {
        let body = "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | Foo | Accepted | 2026-01-01 |
";
        let f = write_tmp(body);
        let idx = parse_index(f.path()).unwrap();
        let out = render_index(&idx);
        assert!(out.contains("| ID  | Title |"));
        assert!(out.contains("| 001 | Foo"));
    }

    #[test]
    fn render_bounded_no_budget_is_unbounded() {
        // Set up a proper repo tree with _index.md so locate_index finds it.
        let dir = tempfile::tempdir().unwrap();
        let decisions = dir.path().join("docs/decisions");
        std::fs::create_dir_all(&decisions).unwrap();
        std::fs::write(
            decisions.join("_index.md"),
            "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | Foo | Accepted | 2026-01-01 |
",
        )
        .unwrap();
        let m = session_manifest_bounded(dir.path(), 5, None).unwrap();
        let out = m.render();
        assert!(out.contains("| 001 | Foo"));
        assert!(!out.contains("truncated"));
    }

    #[test]
    fn render_bounded_truncates_last_file_with_marker() {
        // Build a manifest with 3 large ADR files and a 4 KB budget
        // (4096 bytes is enough to fit the index + the first ADR partial,
        // but not all three).
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let decisions = repo.join("docs/decisions");
        std::fs::create_dir_all(&decisions).unwrap();
        std::fs::write(
            decisions.join("_index.md"),
            "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | First  | Accepted | 2026-01-01 |
| 002 | Second | Accepted | 2026-02-01 |
| 003 | Third  | Accepted | 2026-03-01 |
",
        )
        .unwrap();
        for i in 1..=3 {
            // Each ADR body is ~2 KB so 3 ADRs = 6 KB, exceeding the 4 KB budget.
            let body = format!(
                "# ADR {}\n\nBody line 1 for ADR {}.\n\n{}\n",
                i,
                i,
                "x".repeat(2000)
            );
            std::fs::write(
                decisions.join(format!("{:03}-adr.md", i)),
                body,
            )
            .unwrap();
        }
        let m = session_manifest_bounded(repo, 3, Some(4096)).unwrap();
        let out = m.render_bounded(Some(4096));
        // Must fit the budget (with the truncation marker).
        assert!(
            out.len() <= 4096 + 200,
            "manifest should respect the byte budget (got {} bytes, budget 4096)",
            out.len()
        );
        // Must indicate truncation.
        assert!(
            out.contains("truncated"),
            "truncated output should mention truncation; got: {}",
            out
        );
    }

    #[test]
    fn render_bounded_budget_larger_than_manifest_no_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let decisions = dir.path().join("docs/decisions");
        std::fs::create_dir_all(&decisions).unwrap();
        std::fs::write(
            decisions.join("_index.md"),
            "\
| ID  | Title | Status | Date |
| --- | --- | --- | --- |
| 001 | Foo | Accepted | 2026-01-01 |
",
        )
        .unwrap();
        let m = session_manifest_bounded(dir.path(), 5, Some(1_000_000)).unwrap();
        let out = m.render_bounded(Some(1_000_000));
        assert!(
            !out.contains("truncated"),
            "budget larger than manifest must not produce a truncation marker"
        );
    }

    #[test]
    fn summarize_adr_returns_first_paragraph() {
        let body = "# 014 — JWT validation\n\nStatus: Accepted\n\nWe validate JWTs in the middleware layer using HS256 with a 24h TTL.\n";
        assert!(summarize_adr(body).contains("We validate JWTs"));
    }

    #[test]
    fn summarize_adr_handles_empty() {
        assert_eq!(summarize_adr(""), "");
        assert_eq!(summarize_adr("# only header"), "");
    }

    #[test]
    fn trim_status_drops_completed_section() {
        let body = "\
# Status

Last updated: 2026-04-01

## In Progress

- [ ] Foo (#1)

## Recently Completed

- [x] Bar (#2)

## Next Priorities

1. Baz
";
        let trimmed = trim_status_to_in_progress(body);
        assert!(trimmed.contains("In Progress"));
        assert!(trimmed.contains("Foo (#1)"));
        assert!(!trimmed.contains("Recently Completed"));
        assert!(!trimmed.contains("Bar (#2)"));
        assert!(trimmed.contains("Next Priorities"));
    }

    #[test]
    fn locate_index_prefers_decisions_dir() {
        let dir = tempfile::tempdir().unwrap();
        let decisions = dir.path().join("docs/decisions");
        std::fs::create_dir_all(&decisions).unwrap();
        std::fs::write(
            decisions.join("_index.md"),
            "| ID | Title | Status | Date |\n| --- | --- | --- | --- |\n",
        )
        .unwrap();
        let p = locate_index(dir.path()).unwrap();
        assert!(p.ends_with("docs/decisions/_index.md"));
    }

    #[test]
    fn slugify_handles_punctuation() {
        assert_eq!(slugify("JWT Validation Strategy"), "jwt-validation-strategy");
        assert_eq!(slugify("Repository pattern!"), "repository-pattern");
    }
}