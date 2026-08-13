# Package management

- Always use pnpm over npm.

# Development safety

- Keep automated checks headless. Do not run the GUI unless the host user
  explicitly opts in.

# Seeing the UI

Muxtrix is a native Iced/wgpu app, so a visible change cannot be judged from the
code. Capture a real frame headlessly instead — Xvfb plus a software Vulkan
adapter, driven by the e2e harness. No visible window is ever opened.

- One surface: run the `headless_e2e` test with `MUXTRIX_E2E_SCREENSHOT_RGBA`
  set, plus `MUXTRIX_E2E_CAPTURE` for a named state, `MUXTRIX_E2E_VIEWPORT` for
  a window size, and `MUXTRIX_E2E_SETTINGS` to seed a settings profile. The
  capture states are the branches of `Scenario::stage_capture` in
  `crates/muxtrix-app/src/e2e.rs`; list them with
  `grep -o 'capturing("[a-z-]*")' crates/muxtrix-app/src/e2e.rs | sort -u`.
- Many surfaces: `scripts/capture-gallery` runs the whole capture matrix and
  serves a local review UI. See its README.

A capture failing means the run failed, not that the picture is wrong — the
change broke real interaction flow. Fix that before looking at pixels.
