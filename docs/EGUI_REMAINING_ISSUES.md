# GPUI migration checklist

This document tracks the work required for the GPUI build to replace the Iced build. Keep each item open until its acceptance criteria are met.

## Visual parity and controls

- [x] **Restore frames around unfocused panes.** Unfocused panes retain a strong hairline while the focused pane keeps the accent frame.
  - Focused and unfocused panes both retain a visible frame.
  - Focus remains distinguishable without removing the frame from other panes.

- [x] **Use the correct cursor over dropdowns.** Dropdown controls use the pointer cursor across the closed control and open menu.
  - Hovering and clicking a dropdown uses the cursor expected for an interactive selection control.
  - The cursor remains correct while the dropdown is open and while choosing an option.

- [x] **Increase the Tabs/Agents/Repos toggle height slightly.** The segmented toggle is 26 px high so its labels are vertically centered.
  - All three labels are optically centered at supported interface scales.
  - The adjustment remains compact and does not alter surrounding layout unnecessarily.

- [x] **Increase dropdown control height.** Dropdowns use the standard medium control height and align with adjacent controls.
  - Dropdowns match the height and vertical alignment of adjacent controls.
  - Labels and selected values are vertically centered at supported interface scales.

## Text and keyboard interaction

- [x] **Fix typing in text inputs.** Settings preserves focus on whichever descendant input or selector the user activates.
  - The GitHub Host input accepts typing, editing, selection, and paste.
  - Every other text input is checked for the same failure and fixed at the shared cause.
  - Keyboard focus is visible and remains on the active input until intentionally moved.

- [x] **Add keyboard interaction to dialogs.** Dialogs expose a keyboard button row without intercepting caret movement in text inputs.
  - Left and right arrow keys move selection between dialog buttons.
  - Enter activates the selected button.
  - Escape closes dialogs where a cancel or close action exists.
  - Mouse interaction continues to work unchanged.

- [x] **Select default names when opening naming inputs.** Workspace, tab, and rename inputs select their initial value on open.
  - Typing immediately replaces the selected name without requiring manual deletion.
  - Arrow keys or a pointer click can move the caret and clear the selection for partial edits.
  - Apply the behavior consistently to workspace creation, tab creation, and all rename dialogs.

## Codebase maintenance

- [x] **Refactor the application to idiomatic GPUI.** Flatten the temporary migration module layout and use the established GPUI view conventions directly.
  - Preserve all observable behavior.
  - Reuse existing project conventions rather than introducing parallel abstractions.
  - Keep the refactor reviewable and verify affected interaction paths against the pre-refactor behavior.

- [x] **Remove dead code.** Delete code, dependencies, feature flags, assets, and configuration that are no longer reachable or used after the GPUI migration.
  - Confirm removals across all supported platform and feature configurations before deleting conditional code.
  - Leave no compatibility aliases or obsolete Iced migration paths unless they are still required by a shipped build.

## Build and release

- [x] **Optimize CI/CD performance for the GPUI build.** Measure the new pipeline and remove avoidable repeated work.
  - Determine whether the Zed codebase or its dependencies are cloned or rebuilt on every run.
  - Cache or prebuild stable inputs where correctness and reproducibility permit it.
  - Reorganize repository or job boundaries only where measured results justify the added complexity.
  - Preserve reproducible builds and required Linux, Windows, and macOS checks.
  - Record before-and-after timings for the changed jobs.
  - Baseline run [32633064797](https://github.com/Phoenixmatrix/muxtrix/actions/runs/32633064797) serialized checks (3:22) before Linux (3:56), Windows (6:43), and macOS (3:29) builds, producing a 10:07 job critical path.
  - Optimized run [32646164531](https://github.com/Phoenixmatrix/muxtrix/actions/runs/32646164531) started all four jobs within two seconds: checks 4:22, Linux 3:33, Windows 5:45, and macOS 3:40. The critical path fell to 5:45, a 4:22 (43%) reduction.
  - GPUI/Zed remains a Cargo git dependency; `Swatinem/rust-cache` caches Cargo registry, git, and target data without a second clone or cache convention.

- [x] **Update third-party notices.** Reconcile `THIRD_PARTY_NOTICES.md` with every dependency introduced, removed, or changed by the GPUI build.
  - Include all required copyright and license text.
  - Remove notices that no longer apply.
  - Verify the result against the resolved dependency graph for every shipped target.

- [ ] **Promote the GPUI build to `main`.** The GPUI build is now the preferred implementation and should replace the Iced build on the default branch.
  - Create and push an immutable tag on the final Iced commit before changing `main`.
  - Record the tag name and commit in the release or migration notes.
  - Merge the complete GPUI build and its required CI/release configuration into `main`.
  - Verify branch protection, release, packaging, and installation paths against the new default.
  - Confirm the tagged Iced build remains retrievable and buildable from its documented instructions.

- [ ] **Publish a tagged GPUI release through all three release channels.** Once
  the GPUI build is fully merged into `main`, cut a tagged release and confirm
  that the new build is published everywhere.
  - Create the annotated version tag from the final release commit on `main`
    using the documented release process.
  - Confirm the GitHub release contains the expected Linux, Windows, and macOS
    GPUI build artifacts and checksums.
  - Confirm Scoop, Homebrew, and the apt repository all publish and install the
    same tagged GPUI version.
