---
target: Session picker button affordances and mismatched bulk action sizing
total_score: 27
max_score: 40
na_heuristics: 
p0_count: 0
p1_count: 1
timestamp: 2026-09-05T20-12-45Z
slug: crates-muxtrix-app-src-views-dialogs-rs
---
## Method: dual-agent
A: `ButtonCritiqueDesign` · B: `ButtonCritiqueEvidence`

## Verdict

**Yes—End session and Remove are clickable controls that open confirmations. They should look like buttons.** I styled them as quiet text actions, which makes them resemble the adjacent status labels. That was the wrong treatment here.

The bulk button’s mismatch is also real. The dialog mixes two button specifications:

| Property | End all & start fresh | Start fresh / Resume |
|---|---:|---:|
| Height | 30 px | 30 px |
| Label size | `ui(9)` | `ui(11)` |
| Horizontal padding | 11 px per side | 14 px per side |
| Corner radius | 5 px | 6 px |

The font family is inherited in both cases; **the label size is approximately 18% smaller**, not a different typeface. Although the boxes have equal height, the smaller text and tighter padding make the bulk action look miniature.

Sources: `settings.rs::settings_action_button` and `dialogs.rs::dialog_button`.

## Priority issues

### P1 — One dialog uses incompatible button treatments

**Problem:** End session / Remove have transparent backgrounds and borders at rest, with the same muted text treatment as Running / Stopped. Meanwhile, the bulk button uses smaller settings-page controls beside normal dialog buttons.

**Why it matters:** Users must discover which text is actionable. Around session termination, that ambiguity creates hesitation rather than useful restraint.

**Fix:**
- Give **End session / Remove** visible neutral button surfaces and borders at rest.
- Match the footer buttons’ **30 px height, `ui(11)` label size, 14 px horizontal padding, and 6 px radius**.
- Use a coherent hover and focus treatment across all buttons.
- Keep **Resume** blue. Keep cleanup triggers neutral; reserve strong red styling for the final destructive confirmation.
- Let the bulk button grow horizontally for its label. Do not shrink its typography to make it secondary.

**Suggested command:** `$impeccable polish`.

### P2 — Cleanup buttons lack a discoverable keyboard focus path

**Problem:** Tab moves through the inventory and main footer actions, but not the cleanup controls. Delete and Ctrl+Delete exist, yet the visible instructions teach only arrows and Enter.

**Fix:** Make the selected row’s cleanup action and the bulk button reachable through deliberate keyboard focus. Preserve the shortcuts as accelerators rather than the only keyboard route.

**Suggested command:** `$impeccable harden`.

## What should stay

- **Resume-first hierarchy:** selected session and primary action are clear.
- **Aligned, name-first rows:** the inventory now scans well; no layout redesign is needed.
- **Cancel-default confirmations:** the dangerous operation is clearly explained before execution.

The interface belongs to Muxtrix’s existing native visual system. It needs consistent controls, not a new aesthetic.

## Design health

Scores describe this dialog, not the whole application. Dynamic operations were not re-exercised during this critique.

| Heuristic | Score / 4 | Main observation |
|---|---:|---|
| System status | 3 | Clear selection and Running / Stopped labels |
| Familiar language | 3 | End versus Remove reflects different outcomes |
| User control | 3 | Confirmation cancellation and Escape |
| Consistency | 2 | Mixed button specifications |
| Error prevention | 3 | Confirmation and disabled stopped-session resume |
| Recognition | 2 | Cleanup resembles metadata |
| Efficiency | 3 | Useful but partly undisclosed shortcuts |
| Aesthetic restraint | 3 | Good hierarchy, inconsistent control scale |
| Error recovery | 2 | Errors identify failures but offer limited guidance |
| Contextual help | 3 | Resume guidance is clearer than cleanup guidance |
| **Total** | **27/40** | **Acceptable; button treatment is not finished** |

## User impact

Overall cognitive load is low. The unnecessary effort is **decoding controls**, not choosing among too many options. The flow starts reassuringly, then becomes uncertain when the user wants cleanup.

- **First-time user:** may read “End session” as another information field.
- **Keyboard/low-vision user:** cannot discover cleanup by following the visible controls and Tab sequence.
- **Power user:** gets clear resume shortcuts but must discover cleanup shortcuts elsewhere.

Minor observation: the stopped-session view still advertises “Enter Resume” while Resume is disabled.

## Evidence and limits

Both independent assessments agree on the button mismatch. The detector returned zero findings because **Rust/GPUI is unsupported**—that is not a clean bill of health. Evidence came from source inspection and six existing native headless captures, including compact, light, and large-text states. No browser DOM or overlay applies here, and no new UI changes were made.
