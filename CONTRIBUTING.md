# Contributing to nuvem

Thanks for your interest in improving nuvem. This document explains how to get
a working development setup and what a change needs before it can merge.

## Development setup

The toolchain is pinned in `rust-toolchain.toml`. `rustup` picks it up
automatically, including the `rustfmt` and `clippy` components. You also need:

- `gcloud` (the Google Cloud CLI) on your `PATH` for manual testing. Reads go
  through the native client libraries, but their token comes from
  `gcloud config config-helper`. Configuration changes, SSH, IAP and SSH
  tunnels, plugins and k9s's `get-credentials` step run `gcloud`. A Cloud SQL
  tunnel runs `cloud-sql-proxy`, and k9s runs itself. Lifecycle actions go
  through the client libraries. The test suite needs none of these tools.
- A C compiler and `cmake`, because the TLS stack under the Google Cloud
  clients builds native code.
- `cargo-nextest` (`cargo install cargo-nextest --locked`). CI runs the suite
  under nextest because some tests need per-process isolation.
- `shellcheck` if you change a shell script (`scripts/`, `packaging/homebrew/`).
- `cargo-deny` (`cargo install cargo-deny --locked`) if you change
  dependencies. CI runs it on every push.

The `*_evidence.rs` tests always assert. To also have them write their
rendered frames and transcripts to a directory, set `NUVEM_EVIDENCE_DIR=<dir>`.

## Before you open a PR

Run the quality gates:

```bash
scripts/check-fast.sh   # fmt, clippy -D warnings, shellcheck, test suite, doctests
```

Or the same thing by hand:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --all && cargo test --doc --all
```

CI runs the same checks plus two that the script does not: `cargo doc` under
`-D warnings` and `cargo deny check`. If a change passes
`scripts/check-fast.sh` locally and builds its docs cleanly, we expect it to
pass CI.

How much verification a change needs depends on how far its effects reach:

- Source behavior unchanged (docs, comments, config, test-only, CI
  plumbing): the gates and CI are the whole requirement.
- Behavior changed inside one subsystem: the gates and a review pass.
- Behavior changed across subsystems, or in a sensitive area (new feature,
  security property, cross-cutting refactor): the gates, a review pass and a
  deeper review.

Features ship as small stacked PRs, because a small PR is quicker to review
than the same work in one branch.

The gates enforce some of these conventions, and the docs rely on the rest:

- No clippy warnings. CI runs the command that follows. Keep `--workspace`:
  without it, a warning in `crates/nuvem-domain` is never linted.

  ```bash
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  ```
- Tests for domain logic live in `crates/nuvem-domain`. That crate has its own
  test binary, so you can test it without the rest of the workspace.
- A new integration test file must be mounted in `tests/suite.rs` (or the
  domain crate's `tests/suite.rs`). Per-file test targets are turned off on
  purpose, and the header of that file explains why.
- `ARCHITECTURE.md` is checked by a test. `tests/architecture_layout_test.rs`
  compares its module tree with the source tree in both directions. If you move
  or add a module, update the document in the same change.
- A commit message has a short subject written as a plain sentence, for
  example `Keep the row count when a filter matches nothing`, as in the
  existing history.

## Design decisions

Decisions that shape the architecture, and the evidence behind them, live in
[docs/decisions](docs/decisions). `ARCHITECTURE.md` describes how the code is
arranged today. A decision record says why an alternative was taken or refused,
so that nobody reopens a settled question from a weaker starting point.

## Releasing

Maintainers cut releases by tag. [RELEASING.md](RELEASING.md) lists the steps,
the secrets the release workflow expects, and what to check afterwards.

## Reporting bugs

Open a GitHub issue. For anything that looks like a security problem, see
[SECURITY.md](SECURITY.md) instead of filing a public issue.

For a rendering bug, include the terminal size and the output of
`nuvem info`. Both make the bug much easier to reproduce. The built-in demo
mode (`nuvem --demo`, see the [README](README.md)) reproduces most UI
behavior without any Google Cloud project.

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE) that covers the project.
