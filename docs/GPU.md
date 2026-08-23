# GPU selection and diagnostics

Muxtrix draws through GPUI. Platform backends are DirectX on Windows, Metal on
macOS, and wgpu on Linux, which maps to Vulkan or OpenGL/EGL.

## WSLg

WSLg exposes the Windows GPU to Linux through `/dev/dxg`. On a typical WSL2
host, the hardware graphics route is Mesa Gallium's D3D12 driver rather than
Vulkan:

```text
GPUI / wgpu GL backend
  -> EGL / OpenGL
  -> Mesa Gallium d3d12
  -> /dev/dxg
  -> Windows NVIDIA WDDM driver
  -> a discrete NVIDIA GPU
```

Before GPUI starts, Muxtrix detects WSL and safely re-executes itself with these
process-local defaults when the corresponding variables are not already set:

```text
WGPU_BACKEND=gl
GALLIUM_DRIVER=d3d12
MESA_D3D12_DEFAULT_ADAPTER_NAME=NVIDIA
EGL_LOG_LEVEL=fatal
```

The NVIDIA preference is added only when the WSL NVIDIA driver is mounted.
AMD/Intel-only WSL systems retain Mesa's automatic adapter selection. Any value
set by the user is preserved, including a different backend or adapter.

Mesa's WSL D3D12 path can emit repeated `libEGL` warnings while probing file
descriptors that do not describe render devices. Those probes are non-fatal and
the NVIDIA adapter is still selected. The process-local
`EGL_LOG_LEVEL=fatal` default suppresses that loader noise while retaining fatal
EGL diagnostics. Set `EGL_LOG_LEVEL` explicitly before launch to choose another
verbosity.

Re-execution avoids mutating the environment after Rust or GPUI has started
threads. It changes neither the user's shell configuration nor global WSL
configuration. It also preserves `WAYLAND_DISPLAY`, `DISPLAY`, and
`XDG_SESSION_TYPE`: the accelerated GL/D3D12 route requires WSLg's native
Wayland surface. Forcing that renderer onto XWayland causes
wgpu to reject the surface during startup.

### WSLg resize stability

WSL 2.7.11 with WSLg 1.0.73.2 can crash its own Weston compositor while a
hardware-accelerated Wayland surface is resized continuously. The observable
client error is `Connection reset by peer`; the host evidence is a Weston
SIGSEGV in `libpixman`, not a Muxtrix panic. Muxtrix keeps the accelerated
Wayland path and asks the compositor to resize in terminal-cell increments.
This reduces surface reconfiguration pressure without changing global WSL
settings or affecting native Linux, Windows, macOS, X11, maximized, or tiled
windows. The increments follow the configured terminal metrics.

## Headless probe

Build and run the diagnostic without opening a window:

```sh
cargo run --bin muxtrix-gpu-probe -- \
  --require-hardware \
  --require-adapter NVIDIA
```

The probe applies the same bootstrap defaults, enumerates wgpu adapters, asks
for the high-performance power preference, rejects CPU/`llvmpipe` adapters when
`--require-hardware` is present, and optionally checks the selected adapter's
description.

Expected WSL output on a typical WSL2 host includes:

```text
name="D3D12 (<your GPU>)" backend=Gl type=Other
GPU requirements satisfied.
```

`vulkaninfo` is not an authoritative WSLg acceleration check on this system: it
reports `llvmpipe` because the working hardware path is Mesa D3D12 through
OpenGL/EGL. Use the Muxtrix probe instead.

## GPUI

`docs/GPUI_PORT_PLAN.md` records the port from Iced to GPUI. The retired
Phase 1 spike put the two risky things on screen — the stock `gpui-component`
widgets and a 200x60 monospace grid shaped one line per row — and reported the
adapter and frame cost before its throwaway crate was removed. It established:

### Dependency resolution

`gpui-component` declares its zed dependencies **without a revision**:

```toml
gpui = { version = "0.2.2", git = "https://github.com/zed-industries/zed", features = ["profiler"] }
```

Cargo treats two git dependencies as the same source only when their revision
specs match, and a dependency's own `Cargo.lock` is not honoured by a dependent
workspace. Pinning `rev` on our side while `gpui-component` does not therefore
puts **two different `gpui` crates in one graph**, and the build fails with
type mismatches between them (`expected gpui::app::App, found App`).

So the zed dependencies are declared without a `rev`, and the exact revisions
are pinned in this repo's committed `Cargo.lock`. `cargo update -p gpui` is how
they move. This supersedes the plan's assumption that pinning `gpui-component`
to a revision whose lockfile names a zed revision is enough — it is not.

### Toolchain

`rust-toolchain.toml` is pinned to 1.97.1, matching zed's. `stable` resolved to
1.96.0 locally, which is older than zed's and not guaranteed to build it.

### Linux build dependencies

GPUI links `xkbcommon-x11`. Beyond the historical set, a Linux build needs:

```bash
sudo apt-get install -y libxkbcommon-x11-dev libgl1-mesa-dev
```

Without the first, everything compiles and only the final link fails with
`unable to find library -lxkbcommon-x11`. These have to be added to the CI
image as well.

### Platform matrix

Measured with the now-removed Phase 1 spike: a 200x60 monospace grid, one
`shape_line` per row per frame, beside the stock `gpui-component` widgets.
"Adapter" is what GPUI logs as `Selected GPU adapter`. "Shape" is the cost of
shaping all 60 rows for one frame; the plan's budget is under 1 ms.

| Platform | Adapter | Backend | Frames | Shape |
|---|---|---|---|---|
| Xvfb + llvmpipe (the CI path) | llvmpipe | Vulkan | 54 fps | 310 us |
| WSLg Wayland, `gpu.rs` defaults | **NVIDIA RTX 4080 via D3D12** | GL | 58 fps | 341 us |
| WSLg XWayland, `gpu.rs` defaults | **NVIDIA RTX 4080 via D3D12** | GL | 57 fps | 333 us |
| WSLg Wayland, no `GALLIUM_DRIVER` | llvmpipe (software) | Vulkan | 59 fps | 330 us |
| WSLg Wayland, `d3d12` but no adapter-name hint | llvmpipe (software) | Vulkan | 59 fps | 334 us |
| Windows 11, D3D 11.1 | **NVIDIA RTX 4080** | DirectX | rendered; rate not measured | — |
| Linux X11/Wayland on bare metal | | | not run — no such machine here | |
| macOS | Metal | | built in CI only; not testable here | |

Frame rate is capped by the spike's own 16 ms repaint timer, so ~59 fps is the
ceiling and every row is hitting it. The number that matters is the shaping
cost, and at ~330 us for a full 200x60 grid it is comfortably inside budget
even on the software adapter.

### WSLg: the plan's biggest risk did not materialise

The plan expected GPUI to prefer a `dzn` Vulkan adapter over the working D3D12
GL one, and reserved a patch to `gpui_wgpu` — carried as a vendored fork — to
force the GL backend.

**No patch is needed.** On this machine there is no `dzn` adapter at all. What
actually happens:

- With no `GALLIUM_DRIVER`, Mesa cannot initialise a hardware driver under WSLg
  (`eglInitialize ... DRI2: failed to get driver name`, `ZINK: failed to choose
  pdev`), GPUI enumerates only llvmpipe, and rendering is software.
- With `GALLIUM_DRIVER=d3d12` and `MESA_D3D12_DEFAULT_ADAPTER_NAME=NVIDIA` —
  exactly what `gpu.rs` already sets on WSL — the D3D12 adapter appears and
  GPUI's own scoring picks it, because a non-CPU adapter outranks llvmpipe
  regardless of backend.

So `gpu.rs`'s WSL re-exec carries over to GPUI unchanged and is still
load-bearing. `WGPU_BACKEND=gl` turns out to be unnecessary for adapter choice
— GPUI reaches the right answer on device type alone — but it is harmless and
there is no reason to remove it.

One gap: the adapter-name hint is what rescues hardware selection here, and
`gpu.rs` only sets it when the WSL NVIDIA driver is mounted. Whether Mesa's
automatic D3D12 selection works on an AMD/Intel-only WSL host is untested; this
machine has an NVIDIA GPU.

### Windows

A binary cross-built from this machine with `cargo xwin` runs on Windows 11,
creates its DirectX device on the NVIDIA RTX 4080 at Direct3D 11.1 feature
level, opens a window and renders. The `0x887A002D` error on startup is benign
— it is the DXGI *debug* interface, which needs the optional Graphics Tools
feature installed, and GPUI carries on without it.

Sustained frame rate was not measured there: launched from WSL the process
drew its first frame and then no more. That is consistent with the repaint
behaviour noted below rather than with anything about DirectX, but it means the
Windows shaping cost is still unmeasured. Running the binary from a normal
Windows session, or measuring in CI, would settle it.

Three build constraints found on the way, all of which matter for Phase 7
packaging:

- **A debug GPUI Windows build is not portable off the machine that built it.**
  In debug, `gpui_windows` compiles its HLSL at runtime from
  `env!("CARGO_MANIFEST_DIR")/src/shaders.hlsl` and canonicalises it, so the
  binary carries the build machine's absolute path. Off that machine it fails
  with `Error creating DirectWriteTextSystem ... The system cannot find the
  path specified`, which reads like a font problem and is not one.
- **Shader precompilation is gated on the host OS, not the target.**
  `gpui_windows/build.rs` guards it with `#[cfg(target_os = "windows")]`, which
  in a build script means the machine doing the building. Cross-compiling a
  release Windows binary from Linux therefore never precompiles the shaders.
  Windows releases should keep being built on Windows.
- **`windows-manifest`, a gpui default feature, breaks cross-compilation.** It
  compiles a resource whose manifest path is relative to gpui's own source
  directory; native `rc.exe` resolves that, `llvm-rc` does not. Feature
  unification means it cannot be switched off from this workspace either, since
  `gpui-component` pulls gpui in with default features. A native Windows build
  is unaffected.

### Rendering under Xvfb

GPUI renders under Xvfb on llvmpipe and an X11 `GetImage` of the window returns
the frame — sampled pixels match the colours the spike paints. That is the
mechanism Phase 6 depends on, and it works.

One behaviour to carry into the port: **`cx.notify()` from inside `render` does
not by itself drive a repaint loop.** Under Xvfb the first frame was the only
one until repaints were driven from a timer. Muxtrix already repaints on timers
(cursor blink, the e2e tick), so this costs nothing, but an e2e scenario that
expects a frame purely because state changed needs a tick behind it.

