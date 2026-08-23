# Remaining egui issues

This document tracks the remaining work before the egui build replaces the Iced build. Keep each item open until its acceptance criteria are met.

## Visual parity and controls

- [ ] **Restore frames around unfocused panes.** Panes currently lose their frame when they are not focused, which makes pane boundaries harder to see and differs from the Iced build.
  - Focused and unfocused panes both retain a visible frame.
  - Focus remains distinguishable without removing the frame from other panes.

- [ ] **Use the correct cursor over dropdowns.** Dropdown controls show the wrong pointer type during interaction.
  - Hovering and clicking a dropdown uses the cursor expected for an interactive selection control.
  - The cursor remains correct while the dropdown is open and while choosing an option.

- [ ] **Increase the Tabs/Agents/Repos toggle height slightly.** The segmented toggle is too short for its labels to appear vertically centered.
  - All three labels are optically centered at supported interface scales.
  - The adjustment remains compact and does not alter surrounding layout unnecessarily.

- [ ] **Increase dropdown control height.** Dropdown controls are too small vertically.
  - Dropdowns match the height and vertical alignment of adjacent controls.
  - Labels and selected values are vertically centered at supported interface scales.

## Text and keyboard interaction

- [ ] **Fix typing in text inputs.** The GitHub Host field cannot be typed into, and the same problem appears to affect most text inputs.
  - The GitHub Host input accepts typing, editing, selection, and paste.
  - Every other text input is checked for the same failure and fixed at the shared cause.
  - Keyboard focus is visible and remains on the active input until intentionally moved.

- [ ] **Add keyboard interaction to dialogs.** Keys such as Enter do not work in dialogs, and dialog buttons cannot be selected with the arrow keys.
  - Left and right arrow keys move selection between dialog buttons.
  - Enter activates the selected button.
  - Escape closes dialogs where a cancel or close action exists.
  - Mouse interaction continues to work unchanged.

- [ ] **Select default names when opening naming inputs.** Creating a workspace or tab, and renaming an item, should select the entire default or existing name.
  - Typing immediately replaces the selected name without requiring manual deletion.
  - Arrow keys or a pointer click can move the caret and clear the selection for partial edits.
  - Apply the behavior consistently to workspace creation, tab creation, and all rename dialogs.

## Codebase maintenance

- [ ] **Refactor the application to idiomatic egui.** Remove Iced-specific implementation patterns that remain in the egui build and use established egui patterns instead.
  - Preserve all observable behavior.
  - Reuse existing project conventions rather than introducing parallel abstractions.
  - Keep the refactor reviewable and verify affected interaction paths against the pre-refactor behavior.

- [ ] **Remove dead code.** Delete code, dependencies, feature flags, assets, and configuration that are no longer reachable or used after the egui migration.
  - Confirm removals across all supported platform and feature configurations before deleting conditional code.
  - Leave no compatibility aliases or obsolete Iced migration paths unless they are still required by a shipped build.

## Build and release

- [ ] **Optimize CI/CD performance for the egui build.** Measure the new pipeline and remove avoidable repeated work.
  - Determine whether the Zed codebase or its dependencies are cloned or rebuilt on every run.
  - Cache or prebuild stable inputs where correctness and reproducibility permit it.
  - Reorganize repository or job boundaries only where measured results justify the added complexity.
  - Preserve reproducible builds and required Linux, Windows, and macOS checks.
  - Record before-and-after timings for the changed jobs.

- [ ] **Update third-party notices.** Reconcile `THIRD_PARTY_NOTICES.md` with every dependency introduced, removed, or changed by the egui build.
  - Include all required copyright and license text.
  - Remove notices that no longer apply.
  - Verify the result against the resolved dependency graph for every shipped target.

- [ ] **Promote the egui build to `main`.** The egui build is now the preferred implementation and should replace the Iced build on the default branch.
  - Create and push an immutable tag on the final Iced commit before changing `main`.
  - Record the tag name and commit in the release or migration notes.
  - Merge the complete egui build and its required CI/release configuration into `main`.
  - Verify branch protection, release, packaging, and installation paths against the new default.
  - Confirm the tagged Iced build remains retrievable and buildable from its documented instructions.

- [ ] **Publish a tagged egui release through all three release channels.** Once
  the egui build is fully merged into `main`, cut a tagged release and confirm
  that the new build is published everywhere.
  - Create the annotated version tag from the final release commit on `main`
    using the documented release process.
  - Confirm the GitHub release contains the expected Linux, Windows, and macOS
    egui build artifacts and checksums.
  - Confirm Scoop, Homebrew, and the apt repository all publish and install the
    same tagged egui version.
