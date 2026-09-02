# Muxtrix

Muxtrix is a GPU-accelerated, cross-platform terminal workspace for running and
supervising many concurrent development sessions. It is written primarily in
Rust with [GPUI](https://www.gpui.rs/) for the application UI and Ghostty's VT
engine for terminal emulation.

> **Status: early, and vibe coded.** Muxtrix is pre-1.0 and was built almost
> entirely by prompting AI agents. No attempt was made to make the code good —
> it has not been designed, reviewed, or held to any quality bar, so please
> don't read it as an example of how to write Rust. Expect rough edges and
> breaking changes between releases.

## Installation

Every release ships through a package channel per platform. Each one installs
the `muxtrix` app and the `muxtrixctl` command-line tool.

### Windows (Scoop)

```powershell
scoop bucket add muxtrix https://github.com/Phoenixmatrix/scoop-muxtrix
scoop install muxtrix/muxtrix
```

### Linux (apt)

apt is currently the only supported Linux install channel; there are no
packages for other distributions or package managers yet. Users on non-Debian
systems can grab the tarball from the
[releases page](https://github.com/Phoenixmatrix/muxtrix/releases) instead.

```sh
echo "deb [trusted=yes] https://raw.githubusercontent.com/Phoenixmatrix/muxtrix/apt-repo/ ./" \
  | sudo tee /etc/apt/sources.list.d/muxtrix.list > /dev/null
sudo apt-get update
sudo apt-get install muxtrix
```

### macOS (Homebrew)

Only Apple Silicon (arm64) builds are published; Intel Macs are not supported.

```sh
brew tap phoenixmatrix/muxtrix https://github.com/Phoenixmatrix/homebrew-muxtrix
brew install phoenixmatrix/muxtrix/muxtrix
```

## License

Muxtrix is available under the [MIT License](LICENSE).

Third-party attributions and the exact license texts for Ghostty, Herdr,
libghostty-rs, terminal theme sources, and embedded Unicode data are recorded
in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Release archives and the
Debian package include that file.
