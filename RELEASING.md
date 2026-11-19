# Releasing nuvem

A release is a tag. Pushing `v<version>` runs `.github/workflows/release.yml`,
which builds, tests and attests an archive for each of the four platforms on
GitHub-hosted runners. It then publishes the archives as a GitHub release and
updates the Homebrew tap. You publish to crates.io by hand, after the release.

## What the workflow expects

| Name | Kind | Used by | Without it |
| --- | --- | --- | --- |
| `HOMEBREW_TAP_TOKEN` | secret | the `homebrew` job | the job ends green with a notice and the tap is not updated |

`HOMEBREW_TAP_TOKEN` is a fine-grained personal access token with access to
`filipeaaoliveira/homebrew-tap` only, and the permission Contents: read and
write. Give it an expiry and note the date.

## Steps

1. Bump the version. Set `version` under `[workspace.package]` in the root
   `Cargo.toml`, and set the same version on the `nuvem-domain` dependency in
   that file. Both crates share the workspace version. Then refresh the lock
   file:

   ```bash
   cargo update --workspace
   ```

2. Update the changelog. In `CHANGELOG.md`, rename `## [Unreleased]` to
   `## [<version>] - <YYYY-MM-DD>`, add an empty `## [Unreleased]` above it,
   and add or update the comparison links at the bottom of the file.

3. Check the release. Run the gates in `CONTRIBUTING.md`, then:

   ```bash
   cargo publish --dry-run --workspace --locked
   ```

   This packages both crates and builds each from its package, resolving
   `nuvem` against the local `nuvem-domain` rather than crates.io.

4. Merge and tag. Merge the release commit to `main`, then tag it:

   ```bash
   git tag -a v<version> -m "nuvem <version>"
   git push origin v<version>
   ```

   A tag with a hyphen (`v0.2.0-rc.1`) is published as a pre-release and does
   not update the tap. A tag with build metadata (`+`) does not update the tap
   either.

   The workflow then builds `x86_64-unknown-linux-gnu`,
   `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin` and
   `x86_64-apple-darwin`, each natively on its own runner. It runs the suite
   and a smoke test on each, and creates the release once all four are green.
   Last, it renders `Formula/nuvem.rb` from the published files and pushes it
   to the tap. If the `homebrew` job fails, run it again on its own:

   ```bash
   gh run rerun <run-id> --job <homebrew-job-id>
   ```

5. Publish to crates.io. `nuvem` depends on `nuvem-domain` by version, so
   the domain crate goes first. `cargo publish` waits until a version is in the
   index before it returns.

   ```bash
   cargo publish -p nuvem-domain --locked
   cargo publish -p nuvem --locked
   ```

   A published version cannot be replaced, only yanked. For that reason this
   step is manual, and you run it only after the binaries and the tap are known
   to be good.

## Verify

- The release page lists four archives and four `.sha256` files, and
  `shasum -a 256 -c` passes on each.
- `gh attestation verify nuvem-<target>.tar.gz --repo filipeaaoliveira/nuvem`
  succeeds for each archive.
- The tap has a commit named `nuvem <version>`, and on a Mac
  `brew update && brew upgrade nuvem` (or `brew install filipeaaoliveira/tap/nuvem`)
  installs it. `brew test nuvem` passes and `nuvem info` reports the new
  version.
- `cargo install nuvem --locked` installs the new version.

## Updating the tap by hand

If the `homebrew` job cannot run, render the formula locally from a directory
holding all eight release files, and commit it to the tap as `Formula/nuvem.rb`:

```bash
gh release download v<version> --dir dist --pattern 'nuvem-*'
packaging/homebrew/render-formula.sh <version> dist > nuvem.rb
```
