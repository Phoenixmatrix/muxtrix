# Releasing Muxtrix

One `vMAJOR.MINOR.PATCH` tag drives the GitHub release and every package
repository. Do not publish release assets or update package manifests by hand;
that can leave different platforms on different versions.

## One-time repository setup

Configure these Actions secrets on `Phoenixmatrix/muxtrix`:

- `SCOOP_BUCKET_TOKEN`: a fine-grained token with Contents read/write access to
  `Phoenixmatrix/scoop-muxtrix`.
- `HOMEBREW_TAP_TOKEN`: a fine-grained token with Contents read/write access to
  `Phoenixmatrix/homebrew-muxtrix`.

The workflow's `GITHUB_TOKEN` publishes the GitHub release and the `apt-repo`
branch, so the repository must allow Actions read/write access to contents. The
package repositories must use `main` as their writable default branch, or allow
the Actions bot tokens to push there.

The app repository and its release assets are public. The generated
`apt-repo` branch is therefore directly consumable through
`raw.githubusercontent.com` as documented in `docs/INSTALL.md`.

The public Homebrew repository follows the conventional custom-tap name
`Phoenixmatrix/homebrew-muxtrix` and uses `main` as its default branch. The
workflow creates and maintains `Formula/muxtrix.rb` inside it.

## Cut a release

Update `[workspace.package].version` in `Cargo.toml`, update `Cargo.lock`, and
land that change on `main`. From a clean checkout whose `HEAD` exactly matches
`origin/main`, run:

```sh
scripts/release.sh --dry-run
scripts/release.sh
```

The first command runs the local quality gates without creating a tag. The
second repeats the gates, creates an annotated tag for the Cargo version, and
pushes only that tag. Use `--skip-gates` on the second command only when the
same commit just passed the dry run.

## What the tagged workflow guarantees

Before publishing, the workflow:

1. Runs formatting, Clippy, workspace tests, and the headless application E2E.
2. Builds and packages Linux x64, Windows x64, and macOS arm64 natively where
   appropriate.
3. Verifies the Debian package, archive contents, checksums, tag/version match,
   Windows resources, macOS architecture and ad-hoc signatures, and a local
   Homebrew install/test using the generated formula.
4. Creates one GitHub release containing every platform asset and
   `SHA256SUMS`.
5. Fetches the public macOS release asset through Homebrew and repeats the
   install/test against the published URL.
6. Updates the Scoop manifest, Homebrew formula, and verified flat apt
   repository using that same version and those same asset hashes.

The publication jobs are retry-safe. An existing GitHub release is accepted
only when all immutable assets match the newly generated checksums, and package
repository commits become no-ops when they already contain the expected
version.
