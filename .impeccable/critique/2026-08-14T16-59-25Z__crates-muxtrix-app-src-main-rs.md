---
total_score: 22
max_score: 40
p0_count: 0
p1_count: 2
na_heuristics: ""
target: crates/muxtrix-app/src/main.rs
timestamp: 2026-08-14T16-59-25Z
slug: crates-muxtrix-app-src-main-rs
---
# Muxtrix worktree-agent setup critique

## Method

Dual-agent review of `crates/muxtrix-app/src/main.rs` using the headless native captures `worktree-agent-settings` at 1280x900 and `worktree-agent-setup` at 820x560. Reviewers: `/root/impeccable_critique_a` and `/root/impeccable_critique_b`. The detector was run against the Rust target and repository; its five findings were limited to the out-of-scope capture-gallery HTML and were treated as false positives for this native Iced surface. Browser and DOM review were not applicable.

## Score

| Heuristic | Score | Notes |
|---|---:|---|
| Visual hierarchy | 3/5 | The modal is clear, but the default-agent selector is visually quieter than the integration actions it governs. |
| Layout and spacing | 4/5 | Modal alignment and the 860px settings lane are strong; compact Preferences still needed direct capture evidence. |
| Typography | 3/5 | Existing scale is consistent and readable, though dense integration metadata competes with action labels. |
| Color and contrast | 3/5 | Status colors are coherent; the selector's lavender treatment feels slightly detached from the surrounding settings system. |
| Interaction clarity | 2/5 | The requested command was discarded when opening Settings, so setup did not fulfill the user's original action. |
| Consistency and state | 2/5 | Installed rows exposed Add, Remove, Re-add, and Launch simultaneously despite those actions being mutually exclusive by state. |
| Responsive behavior | 2/5 | Full-width alignment was verified, but compact settings had not been captured and the four-button rows were likely to compress poorly. |
| Content and guidance | 3/5 | The dialog explains the requirement, but did not distinguish missing, unselected, and broken-default states or identify the actual chosen agent in the palette. |
| **Total** | **22/40** | |

## Findings

### P1 - Preserve and resume the user's command

Opening Settings from the setup dialog must retain the exact right, down, new-worktree restart, or existing-worktree restart command. After a valid default is saved, Muxtrix should continue that command. Cancel and Escape should explicitly cancel it.

### P1 - Make integration actions state-specific

An installed integration should show Launch and Remove hooks; an uninstalled integration should show Add integration; a stale integration should show Repair hooks and Remove hooks. Busy rows should not expose actionable controls. Copy must clarify that hook actions happen immediately while preference changes are saved with Apply.

### P2 - Make setup guidance state-aware

Use distinct copy for no configured integrations, configured integrations without a default, and a saved default whose hooks need repair. The primary button should describe the applicable next step and the secondary action should say that it cancels the pending command.

### P2 - Name the selected agent in the palette

Once a valid default exists, replace generic “with agent” and “default agent” language with Codex, Claude Code, or Oh My Pi so the command's effect is explicit before activation.

### P2 - Verify compact settings visually

Capture the Preferences surface at a narrow supported viewport after simplifying the action rows. Check label/control alignment, button wrapping, footer density, and the default-agent picker.

## Strengths to preserve

- The setup modal has strong centered composition, spacing, and button alignment.
- The deep link lands directly at the lifecycle-hook settings section.
- The default picker only offers integrations whose user-level lifecycle hooks are configured.
- The feature is discoverable in the command palette while still gated safely.
