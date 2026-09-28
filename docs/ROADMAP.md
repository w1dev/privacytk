# Roadmap

Planned improvements that are **documented but not implemented** in
this fork. Each item lists acceptance criteria so a future round can
pick it up without re-reading the full CORPORATE.md section it came
from.

The fork is correct and shippable as of the last commit. These are
*additive* improvements, not regressions.

---

## CFD context quality for large decision repositories

**Trigger**: today, `pkt cfd-context --top-adrs N` ranks by date
descending. For repos with 20+ ADRs, the top-N is dominated by recent
changes — often irrelevant to the LLM's current task. The bundle
becomes background noise that consumes tokens without informing the
decision.

**Acceptance criteria** (apply to all three items below):

- No regression on small repos (≤10 ADRs): bundle contents and
  ranking unchanged.
- Operator can opt in via flag (`--enable-importance` or
  `--enable-embeddings`), not hardcoded behavior change.
- Drift test asserts the flag is absent today and present after
  implementation (mirrors the pre-commit gate pattern).
- README FAQ and `CORPORATE.md` §2c are updated to describe the
  new mode.

### Item 1 — Manifest + tools API

**Idea**: `pkt cfd-context --manifest-and-tools` emits the manifest
plus a JSON tool descriptor so the LLM invokes `cfd-read` for ADRs
it actually needs instead of receiving them inline.

**Acceptance**:

- New CLI flag `--manifest-and-tools` on `pkt cfd-context`.
- Output: `{ "manifest": "<markdown>", "tools": [{ "name":
  "cfd-read", "description": "Read full ADR content by id",
  "args": { "id": "string", "format": "full|summary" } }] }`.
- The default (no flag) preserves today's output for back-compat.
- Drift test in `src/core/drift_check.rs` asserts
  `manifest_and_tools_flag_exists`.
- `CORPORATE.md` §2c gets a "Manifest + tools" subsection with
  example output and rationale.
- README FAQ ("What if the bundle truncates…") references this flag
  in option 3.

**Effort**: 4–6 hours (code: 2h, tests: 1h, drift test: 0.5h, docs:
1h, smoke: 0.5h).

**Why first**: it's the cleanest pattern (matches how Claude Code
already consumes tools on demand), and unblocks the design space for
items 2 and 3.

### Item 2 — Per-ADR importance scoring

**Idea**: rank ADRs by a combination of recency, decisiveness
(`Accepted` > `Proposed` > `Superseded`), and a heuristic topic
relevance derived from the LLM's most recent Bash commands.

**Acceptance**:

- New CLI flag `--enable-importance` on `pkt cfd-context`.
- Scoring function: `score(adr) = 0.5 * recency_norm +
  0.3 * decisiveness_norm + 0.2 * topic_relevance_norm`.
- Topic relevance uses tokens from the LLM's most recent Bash
  commands (collected via the cfd-context-injector's parser).
- Test fixtures: ADRs of varying status and date produce a
  reproducible ranking.
- The default (no flag) preserves today's date-descending order for
  back-compat.
- Drift test in `src/core/drift_check.rs` asserts the scoring
  function is only called when the flag is set.

**Effort**: 2–3 hours.

**Why second**: smaller scope, depends on item 1's tool surface for
the topic-relevance signal but is not blocked by it.

### Item 3 — Embedding-based similarity

**Idea**: for repos with 50+ ADRs, compute cosine similarity between
the LLM's recent task description (extracted from CLAUDE.md +
CURRENT_STATUS.md) and each ADR's title/summary, pick top 5.

**Acceptance**:

- New CLI flag `--enable-embeddings`.
- Adds `fastembed` or `ort` as a dependency (TBD by ablation).
- The first model load warms a model cache (`~/.cache/pkt/models`)
  so subsequent sessions have zero startup cost.
- Latency budget: ≤200ms added per `cfd-context` call after warm.
- Falls back to importance scoring (item 2) on model failure.
- Drift test asserts the model cache path and that no model is
  downloaded during `cargo test` (tests run with the fallback).
- `Cargo.toml` size delta documented (target: <5MB added to binary).

**Effort**: 6–8 hours.

**Why third**: requires a dep that ships a model artifact; the
maintenance burden (model updates, license review, offline support)
is real. Only worth it for repos where items 1 and 2 still produce
noise.

---

## Intake process for future improvements

When you (or a future maintainer) want to add a feature:

1. Open an issue with the template from
   `.github/PULL_REQUEST_TEMPLATE.md`.
2. Reference this file by item number.
3. Implement to the acceptance criteria. If criteria shift,
   update them in this same commit so ROADMAP.md stays the source
   of truth.

---

*Last reviewed: 2026-09-27. All items above are not started; the
fork is shippable without them.*
