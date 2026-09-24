---
target: desktop notification settings
total_score: 21
max_score: 40
na_heuristics: 
p0_count: 0
p1_count: 3
timestamp: 2026-09-24T20-14-39Z
slug: crates-muxtrix-app-src-views-settings-rs
---
Method: dual-agent (A: design review · B: detector)

## Design Health Score (before fixes)

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 1 | Send test gave no feedback; delivery errors went to stderr |
| 2 | Match System / Real World | 3 | "pane's marker" was internal jargon |
| 3 | User Control and Freedom | 3 | Notices withdraw on resume/focus; clicks keep unsaved settings |
| 4 | Consistency and Standards | 2 | Test claimed "notifications are on" regardless of draft/applied state |
| 5 | Error Prevention | 2 | Errors shared the low-urgency "finishes" toggle |
| 6 | Recognition Rather Than Recall | 3 | Rows self-explanatory |
| 7 | Flexibility and Efficiency | 2 | No per-agent/workspace scope, snooze, or palette toggle |
| 8 | Aesthetic and Minimalist Design | 3 | Follows preference-table grammar; dense footnote |
| 9 | Error Recovery | 0 | Denied permission / missing daemon surfaced nowhere |
| 10 | Help and Documentation | 2 | No platform prerequisites explained |
| **Total** | | **21/40** | Acceptable |

## Design Specificity Verdict
LLM: notice copy and lifecycle (per-pane replace, withdraw on resume/focus, cooldown, taskbar attention) are authored for Muxtrix; the settings surface was weaker than the logic behind it.
Detector: detect.mjs does not scan .rs (SCANNABLE_EXTENSIONS excludes it); exit 0 with [] is no evidence. Browser overlay n/a for native GPUI.

## Priority Issues
- [P1] Test notice could confirm something false → retitled "Test from Muxtrix / Agent notifications will look like this". FIXED
- [P1] Silent delivery failure → Linux D-Bus errors and unbundled macOS reported inline under Send test (danger tone) and in status for real notices. FIXED (Windows: GPUI swallows toast errors)
- [P1] Send test clipped at 820px → buttons flex_shrink_0, copy min_w 0 (also fixed pre-existing hooks Refresh overflow). FIXED
- [P2] Failed shared the "finishes" toggle → errors now ride with "When an agent needs you". FIXED
- [P3] Footnote jargon → "Sent only while Muxtrix is in the background. Click one to open its pane." FIXED

## Persona Red Flags
- Power user: no per-pane mute, snooze, or palette toggle.
- First-timer: Apply vs. test distinction; now the test copy makes no claim.
- Windows+WSL dev with 6 agents: burst of "finished" toasts; consider coalescing.

## Minor Observations
- Section noun unified ("System notifications").
- Compact footer "Apply changes" clips at 820px (pre-existing, out of scope).

## Questions to Consider
- Should a focused window still notify when the pane is in another workspace?
- Should "finishes" be a digest so desktop notices mean only "blocked"?
