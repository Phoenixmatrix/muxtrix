---
target: GitHub diff view and sidebar loading
total_score: 28
max_score: 40
na_heuristics:
p0_count: 0
p1_count: 2
timestamp: 2026-08-13T21-12-27Z
slug: crates-muxtrix-app-src-main-rs
---
# GitHub review surface critique

## Design Health Score

| # | Heuristic | Score | Key issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 2/4 | Initial load has truthful copy but no visible motion; refresh with cached data leaves stale review evidence visible. |
| 2 | Match System / Real World | 3/4 | The diff grammar matches developer conventions, but unconditional horizontal scrolling diverges from GitHub-style reading on wide surfaces. |
| 3 | User Control and Freedom | 3/4 | Back, Escape, Close, and retained file navigation are strong; stale actions should disappear during a full refresh. |
| 4 | Consistency and Standards | 3/4 | The ledger follows Muxtrix tokens and ruled-row grammar; the static refresh tile does not read as active loading. |
| 5 | Error Prevention | 2/4 | Merge head guards are strong, but stale merge and file controls remain exposed while review state reloads. |
| 6 | Recognition Rather Than Recall | 3/4 | Diff semantics are familiar; long lines make users repeatedly find and manage a distant horizontal scrollbar. |
| 7 | Flexibility and Efficiency | 3/4 | Virtualization, retained navigation, and Escape support expert work; wide no-wrap diffs add avoidable two-axis navigation. |
| 8 | Aesthetic and Minimalist Design | 3/4 | The surface is calm and dense, but clipped long lines and a persistent horizontal bar weaken the wide composition. |
| 9 | Error Recovery | 3/4 | Failures have direct explanations and retry paths; refresh failure with cached data needs an unambiguous transition. |
| 10 | Help and Documentation | 3/4 | Inline explanations and tooltips are appropriate for a professional tool. |
| **Total** | | **28/40** | **Good foundation; wrapping and loading-state trust need hardening.** |

## Design Specificity Verdict

The full-window code review surface, retained 372 px review ledger, semantic merge-readiness colors, and continuous changed-file rows feel authored for Muxtrix. Familiar diff conventions are appropriate in Operate mode. The weakest, most interchangeable element is loading: a static refresh glyph in a raised tile reads like a generic empty-state illustration instead of live native-tool feedback.

The deterministic Impeccable detector reported zero findings for `crates/muxtrix-app/src/main.rs`. That clean result has limited coverage because the target is native Rust/Iced rather than DOM markup. Native headless screenshots supplied the relevant visual evidence: at 1280x800 long lines remain single-line and require horizontal scrolling despite a useful reading lane; at 720x480 horizontal scrolling is justified because the retained ledger leaves a narrow code surface. Browser overlays were unavailable for the native app.

## Overall Impression

The review flow already feels focused, trustworthy, and product-specific. Its biggest opportunity is to make width and asynchronous state behave like the code-review tool users already understand: wrap when the lane can sustain an 80-column reading measure, scroll when it cannot, and replace stale review evidence with unmistakable loading feedback during full refreshes.

## What's Working

1. The decision-to-evidence hierarchy is excellent: readiness, PR identity, checks, then changed files.
2. The retained file ledger keeps repository context and selection visible while the diff owns the primary surface.
3. The compact 720x480 header preserves Back, path, status, and stats without changing the desktop interaction model.

## Priority Issues

### P1 - Full refresh is visually ambiguous and leaves stale controls exposed

The panel renders cached data before its loading state, so refresh can leave Merge, readiness, and file actions looking current while replacement data is in flight. Make loading the authenticated body's top-level state. Preserve repository identity and Close, hide Refresh and all review evidence, and use a fixed-footprint animated 3x3 nine-dot loader with explicit Reading or Refreshing copy.

Suggested command: `$impeccable harden`

### P1 - Diff wrapping ignores the readable viewport

The document expands to its longest line, every row uses no-wrap text, and the scroller always enables both axes. Compute the real text lane after gutters and padding. At 80 configured terminal cells or wider, wrap within the lane and remove horizontal scrolling. Below that threshold, preserve the existing horizontal behavior.

Suggested command: `$impeccable adapt`

### P2 - Wrapping must preserve virtualization and scroll stability

One logical line currently equals one fixed 24 px row. Naive widget wrapping would break spacers and scrolling. Build a bounded visual-row layout or cached prefix counts so continuation rows keep blank gutters, semantic backgrounds, and stable virtualization. Recompute predictably across the threshold and under font scaling.

Suggested command: `$impeccable optimize`

### P2 - Loading motion needs native-tool discipline

Use motion to communicate activity, not decorate it. Keep all nine dots in a stable 3x3 footprint, animate one accent head with a short fading trail, and retain text so motion and color are not the only signals. The lightweight refocus probe should remain silent.

Suggested command: `$impeccable animate`

### P2 - Capture coverage misses the risky transitions

Add deterministic capture states for initial loading, refresh with cached data, wide wrapped diff, and below-threshold horizontal diff. Verify that Refresh, Merge, and files are absent during full reload while Close remains available.

Suggested command: `$impeccable audit`

## Persona Red Flags

**Alex, power reviewer:** Wide-screen review still demands horizontal panning. During refresh, unchanged green readiness appears authoritative even though remote state is being replaced.

**Jordan, first-time contributor:** The static boxed refresh glyph can look like a disabled button. An unchanged ledger after Refresh gives no immediate confirmation that the click worked.

**Sam, keyboard and low-vision user:** Repeatedly acquiring a distant horizontal scrollbar is costly. The loader must keep textual status and stable contrast, and removing actions during loading must not trap the user; Close remains available.

## Minor Observations

- Keep blank line-number gutters on wrapped continuation rows so one logical line remains recognizable.
- The minimum-width capture justifies horizontal scrolling below the threshold; wrapping there would fragment code.
- Do not animate the lightweight refocus PR probe; ordinary focus switching should remain quiet.
- If refresh fails after cached data existed, show an explicit error and retry state instead of silently restoring apparently current evidence.

## Questions to Consider

- Is 80 characters a raw breakpoint or a promise tied to the configured terminal face and scale? It should use measured terminal-cell width after gutters.
- What proves Refresh worked? The immediate transition from actionable evidence to a distinct nine-dot loading body, followed by one coherent replacement.
- Which shell should remain stable while evidence reloads? Repository identity and Close preserve context and freedom without presenting stale claims.
