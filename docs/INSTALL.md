# Installing Muxtrix

Versioned releases publish Linux x64, Windows x64, and macOS arm64 binaries
plus a Debian package. The macOS binaries are ad-hoc signed but are not
Developer ID signed or notarized; all other builds are unsigned. Verify
downloads against the `SHA256SUMS` file published with each release.

Linux x64 releases target glibc 2.34, which covers Ubuntu 22.04 LTS and newer
and avoids tying packages to the distribution used to build a release.

## macOS — Homebrew

The custom tap installs the native Apple Silicon build and its command-line
control helper. The application and its release assets are public, so the
conventional tap needs no download credentials:

```sh
brew tap phoenixmatrix/muxtrix
brew install phoenixmatrix/muxtrix/muxtrix
```

The current Homebrew package is arm64-only and requires macOS Monterey or
newer. Run the app from a terminal with `muxtrix`.

## Windows — Scoop

The application and its release assets are public, while the Scoop bucket is
private. Install Git with Scoop and add the bucket over SSH:

```powershell
scoop install git
scoop bucket add muxtrix git@github.com:Phoenixmatrix/scoop-muxtrix.git
scoop install muxtrix/muxtrix
```

## Debian/Ubuntu — apt repository

Each tagged release regenerates the unsigned flat repository on the
`apt-repo` branch and exercises an isolated apt client against it before the
branch is updated. Add the public branch as a trusted unsigned apt source:

```sh
echo 'deb [trusted=yes] https://raw.githubusercontent.com/Phoenixmatrix/muxtrix/apt-repo/ ./' \
  | sudo tee /etc/apt/sources.list.d/muxtrix.list
sudo apt-get update
sudo apt-get install muxtrix
```

## Debian/Ubuntu — direct .deb package

Download `muxtrix_<version>_amd64.deb` from the
[latest release](https://github.com/Phoenixmatrix/muxtrix/releases/latest) and
install it directly. `apt` resolves dependencies for local files:

```sh
gh release download --repo Phoenixmatrix/muxtrix --pattern 'muxtrix_*_amd64.deb'
sudo apt-get install ./muxtrix_<version>_amd64.deb
```

The Debian package installs a desktop launcher and its Muxtrix icon alongside
`muxtrix` and `muxtrixctl`, so desktop environments can discover it without a
per-user launcher.

## Direct release archives

Every release carries plain archives (`.tar.gz` for Linux and macOS, `.zip` for
Windows) containing `muxtrix` and `muxtrixctl`. Unpack one and put both
binaries somewhere on your `PATH`.

Verify the download first:

```sh
sha256sum -c SHA256SUMS --ignore-missing
```

## Diagnostics

Run the built-in headless diagnostic when Muxtrix cannot start or selects an
unexpected graphics adapter:

```sh
muxtrix doctor
```

The report identifies the executable and version, validates the settings and
session paths, shows the effective graphics environment, and enumerates the
wgpu adapters using the application's selection policy. It contains no
environment values beyond the four documented graphics overrides and uploads
nothing. Warnings keep a zero exit status; failed required checks return status
1.

## From source

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the full build instructions,
including the pinned Zig toolchain and the Linux development packages the GPU
backend needs.
