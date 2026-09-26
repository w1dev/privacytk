# Analytics

> privacytk corporate fork of [rtk-ai/rtk](https://github.com/rtk-ai/rtk) v0.49.0.
> For the fork's architecture see [CLAUDE.md](../../CLAUDE.md) and
> [docs/CORPORATE.md](../../docs/CORPORATE.md).

## Scope

**Read-only dashboards** over the tracking database. Queries the recorded bash output reduction, correlates with external spending data, and surfaces adoption metrics. Never modifies the tracking DB.

Note: in privacytk the `cc-economics` command does not invoke `npx ccusage` (that fallback hit the npm registry and was removed in the corporate fork).

Owns: `privacytk gain` (savings dashboard) and `privacytk session` (adoption analysis).

Does **not** own: recording token savings (that's `core/tracking` called by `cmds/`), or command filtering itself (that's `cmds/`).

Boundary rule: if a new module writes to the DB, it belongs in `core/` or `cmds/`, not here. Tool-specific analytics (like `cc_economics` reading Claude Code data) are fine — the boundary is "read-only presentation", not "tool-agnostic".

## Purpose
Bash output reduction analytics, economic modeling, and adoption metrics. The stored percentages measure output bytes; token counts are `bytes / 4` estimates, not billed tokens.

These modules read from the SQLite tracking database to produce dashboards, spending estimates, and session-level adoption reports.

## Adding New Functionality
To add a new analytics view: (1) create a new `*_cmd.rs` file in this directory, (2) query `core/tracking` for the metrics you need using the existing `TrackingDb` API, (3) register the command in `main.rs` under the `Commands` enum, and (4) add `#[cfg(test)]` unit tests with sample tracking data. Analytics modules should be read-only against the tracking database and never modify it.
