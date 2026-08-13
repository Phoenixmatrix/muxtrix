# Installing Muxtrix

Versioned releases publish Linux x64 and Windows x64 binaries plus a Debian
package. All builds are unsigned — verify downloads against the `SHA256SUMS`
file published with each release.

macOS is not yet published. The codebase targets macOS, but no macOS binary is
built and the platform is untested; build from source if you want to try it.

Linux x64 releases target glibc 2.34, which covers Ubuntu 22.04 LTS and newer
and avoids tying packages to the distribution used to build a release.

## Debian/Ubuntu — .deb package

Download `muxtrix_<version>_amd64.deb` from the
[latest release](https://github.com/Phoenixmatrix/muxtrix/releases/latest) and
install it directly. `apt` resolves dependencies for local files:

```sh
sudo apt-get install ./muxtrix_<version>_amd64.deb
```

The Debian package installs a desktop launcher and its Muxtrix icon alongside
`muxtrix` and `muxtrixctl`, so desktop environments can discover it without a
per-user launcher.

## Linux and Windows — release archive

Every release carries plain archives (`.tar.gz` for Linux, `.zip` for Windows)
containing `muxtrix` and `muxtrixctl`. Unpack one and put both binaries
somewhere on your `PATH`.

Verify the download first:

```sh
sha256sum -c SHA256SUMS --ignore-missing
```

## From source

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the full build instructions,
including the pinned Zig toolchain and the Linux development packages the GPU
backend needs.
