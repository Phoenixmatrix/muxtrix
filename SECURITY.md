# Security Policy

## Supported versions

Only the latest release receives security fixes. Muxtrix is pre-1.0 and there
are no maintained release branches.

## Reporting a vulnerability

Please report security issues privately through
[GitHub Security Advisories](https://github.com/Phoenixmatrix/muxtrix/security/advisories/new)
rather than opening a public issue.

Muxtrix is maintained by one person, so expect a best-effort response rather
than a guaranteed timeline. I will acknowledge reports as quickly as I can and
will tell you plainly if something will take a while or if I decide not to fix
it.

## Scope worth knowing about

Three areas of the codebase handle untrusted or privileged input and are the
most likely places for a real vulnerability:

- **Terminal emulation.** Muxtrix parses arbitrary VT output from whatever runs
  in a pane, including escape sequences, OSC commands, and clipboard requests.
- **The control service.** `muxtrixctl` talks to the application over a
  user-local Unix socket or Windows named pipe. It is not exposed to the
  network, but it does accept commands that act on live panes. See
  [docs/CONTROL.md](docs/CONTROL.md).
- **Agent lifecycle hooks.** Opt-in integrations write hook configuration for
  Codex and Claude Code and bridge events across the Windows/WSL2 boundary. See
  [docs/AGENT_INTEGRATIONS.md](docs/AGENT_INTEGRATIONS.md).

## Release integrity

Release binaries are **unsigned**. Each release publishes a `SHA256SUMS` file;
verify your download against it. If you need stronger guarantees, build from
source — see [CONTRIBUTING.md](CONTRIBUTING.md).
