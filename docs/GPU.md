# GPU selection and diagnostics

Muxtrix uses Iced's wgpu compositor. Platform backends are Direct3D 12 on
Windows, Metal on macOS, and Vulkan or OpenGL/EGL on native Linux.

## WSLg

WSLg exposes the Windows GPU to Linux through `/dev/dxg`. On a typical WSL2
host, the hardware graphics route is Mesa Gallium's D3D12 driver rather than
Vulkan:

```text
Iced / wgpu GL backend
  -> EGL / OpenGL
  -> Mesa Gallium d3d12
  -> /dev/dxg
  -> Windows NVIDIA WDDM driver
  -> a discrete NVIDIA GPU
```

Before Iced starts, Muxtrix detects WSL and safely re-executes itself with these
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

Re-execution avoids mutating the environment after Rust or Iced has started
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

The probe applies the same bootstrap defaults, enumerates wgpu adapters, uses
Iced's high-performance power preference, rejects CPU/`llvmpipe` adapters when
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
