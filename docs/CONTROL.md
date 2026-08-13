# Local control API

Muxtrix exposes the same typed actions used by its UI through `muxtrixctl`.
The app owns a user-local endpoint: a mode-0600 Unix socket on Linux/macOS and
a local named pipe on Windows. Requests and responses are newline-delimited
JSON. The transport is intentionally local-only; remote access is not exposed.

## Commands

Build the app and control client together:

```sh
cargo build -p muxtrix --bin muxtrix
cargo build -p muxtrix-control --bin muxtrixctl
```

With Muxtrix running:

```sh
muxtrixctl ping
muxtrixctl panes
muxtrixctl split right
muxtrixctl split down
muxtrixctl focus PANE_UUID
muxtrixctl close --pane PANE_UUID
muxtrixctl send $'cargo test\r' \
  --pane PANE_UUID
muxtrixctl capture --pane PANE_UUID
muxtrixctl notify --title "Build" --body "Ready for review" --pane PANE_UUID
muxtrixctl launch codex
muxtrixctl launch claude
```

`MUXTRIX_PANE_ID` is injected into every terminal. Commands and lifecycle hooks
use it as the default pane when `--pane` is omitted. The app responds to every
request with structured JSON and rejects unknown pane IDs without changing the
workspace. Keep `--pane` and its value in the same shell command; a trailing
`--pane` is rejected rather than silently falling back to the focused pane.

## Endpoint discovery

- Linux/macOS: `$XDG_RUNTIME_DIR/muxtrix.sock`, or a private mode-0700
  per-user directory below the system temporary directory when no runtime
  directory is available.
- Windows: a per-user local named pipe name.
- Tests and diagnostics: `MUXTRIX_CONTROL_ENDPOINT` overrides discovery.

The Unix socket itself is mode 0600. The API currently relies on local OS
endpoint permissions and is not a network or multi-user protocol. Authentication
and protocol version negotiation are required before any remote transport is
introduced.

## Windows application with WSL sessions

Windows Muxtrix launches explicit WSL profiles through `wsl.exe`. It shares
`MUXTRIX_PANE_ID` and the GUI server's exact endpoint through `WSLENV`, preserving
the user's existing `WSLENV` entries. This lets a hook running in Linux call the
Windows `muxtrixctl.exe` through WSL interoperability and reach the native
Windows named pipe.

Install the hook from inside WSL and point it at the Windows executable as
described in [AGENT_INTEGRATIONS.md](AGENT_INTEGRATIONS.md). Muxtrix does not
modify global WSL configuration and does not require a TCP bridge.
