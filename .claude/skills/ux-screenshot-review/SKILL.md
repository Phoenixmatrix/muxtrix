---
name: ux-screenshot-review
description: Use after making any visible UI/UX change to Muxtrix (sidebar, fleet, tab strip, pane chrome, dialogs, palette, settings, colors, spacing, typography). Captures real rendered frames headlessly via the e2e harness, verifies the change visually, and then runs the impeccable design review on the result. Also use when the user asks for an app screenshot.
---

Muxtrix is a native Iced/wgpu app, so UI changes cannot be judged from code
alone — and this repo forbids launching the visible GUI from automation. The
sanctioned path is the headless e2e harness: it boots the real binary under a
private Xvfb display with a software Vulkan adapter, drives the full
interaction scenario, and dumps the final GPU frame.

## 1. Capture screenshots

After the change compiles and tests pass, capture the states your change
touched:

```bash
.claude/skills/ux-screenshot-review/scripts/capture.sh <scratch>/workspace.png
.claude/skills/ux-screenshot-review/scripts/capture.sh <scratch>/settings.png --capture settings
.claude/skills/ux-screenshot-review/scripts/capture.sh <scratch>/palette.png --capture palette
.claude/skills/ux-screenshot-review/scripts/capture.sh <scratch>/compact.png --viewport 820x560
```

- Write PNGs to the session scratchpad, not the repo.
- Default viewport is 1280x800; 820x560 exercises the compact/collapsed-rail
  layout. The minimum supported window is 720x480.
- `--capture <state>` ends the scenario on a named surface instead of the
  workspace view. Every name is one branch of `Scenario::stage_capture` in
  `crates/muxtrix-app/src/e2e.rs`; list the current set with
  `grep -o 'capturing("[a-z-]*")' crates/muxtrix-app/src/e2e.rs | sort -u`.
  A state that isn't there yet (a new dialog, an attention outline) needs a
  branch added there rather than screenshotting by hand.
- `MUXTRIX_E2E_SETTINGS=<profile.json>` seeds the settings file before boot,
  which is how a capture pins a theme, font, weight, type size, appearance, or
  fleet view. `capture.sh` passes the variable through from the environment.
- Each capture runs the full e2e (~10-30 s). The run failing means the UI
  change broke real interaction flow — fix that before looking at pixels.

## 1b. Review many states at once

For a broad sweep rather than one changed surface, `scripts/capture-gallery`
drives the whole capture matrix and serves a local review UI. It is ordinary
repo tooling, not part of this skill:

```bash
cd scripts/capture-gallery
bun build.mjs             # ~165 states, 5 parallel jobs, ~4 min
bun server.mjs            # → http://localhost:5173
```

Output and review verdicts land in `~/.muxtrix/capture-gallery`, outside the
repo. The gallery opens on the states never reviewed plus the ones whose pixels
changed since their last verdict, so a second pass only shows what moved. See
`scripts/capture-gallery/README.md`.

## 2. Verify the change yourself

Open every PNG with the Read tool and check, at minimum:

- The changed element looks as intended (alignment, spacing, centering,
  color) and did not regress its neighbors.
- Both the default and compact viewports when the change touches the rail,
  tab strip, or pane headers — density rules differ between them.
- Terminal content still dominates; chrome stays quiet (see DESIGN.md).

If the user should see the result, send the PNG with SendUserFile.

## 3. Run the impeccable review

Once the screenshots confirm the change works mechanically, review the craft:
invoke the `impeccable` skill and follow its review flow for the
changed surface (typically its critique/review command for refinements). Give
it the captured screenshots as evidence alongside the code. Fix what the
review surfaces in one batch, recapture only the affected states, and confirm
with at most one more round — do not loop.

Update DESIGN.md when the change alters documented layout, tokens, or
interaction grammar.
