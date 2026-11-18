# Contributing to nuvem

Thanks for your interest in improving nuvem. This document explains how to get
a working development setup and what a change needs before it can merge.

## Development setup

The toolchain is pinned in `rust-toolchain.toml`; `rustup` picks it up
automatically, including the `rustfmt` and `clippy` components. Beyond that
you need:

- `gcloud` (the Google Cloud CLI) on your `PATH` for manual testing — reads go
  through the native client libraries, but their token comes from
  `gcloud config config-helper`, and every mutation, tunnel and plugin is still
  a `gcloud` invocation. The test suite does not need it.
- A C compiler and `cmake`: the TLS stack under the Google Cloud clients builds
  native code.
- `cargo-nextest` (`cargo install cargo-nextest --locked`) — CI runs the suite
  under nextest because some tests need per-process isolation.
- `shellcheck` if you touch a shell script (`scripts/`, `packaging/homebrew/`).
- `cargo-deny` (`cargo install cargo-deny --locked`) if you touch
  dependencies; CI runs it on every push.

The `*_evidence.rs` tests always assert; set `NUVEM_EVIDENCE_DIR=<dir>` to
also have them write their rendered frames and transcripts there.

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

CI runs the same checks plus two it does not: `cargo doc` under `-D warnings`
and `cargo deny check`. A change that passes `scripts/check-fast.sh` locally
and builds its docs cleanly should pass CI.

How much verification a change needs depends on what it puts at risk:

- **Source behaviour unchanged** (docs, comments, config, test-only, CI
  plumbing): the gates plus CI are the whole requirement.
- **Behaviour changed, blast radius local** (one subsystem): the gates plus a
  review pass.
- **Behaviour changed, blast radius wide or sensitive** (new feature, security
  property, cross-cutting refactor): the above plus a deeper review.

Features ship as small stacked PRs: four small PRs each review in minutes,
where the same work in one branch takes hours.

A few conventions the gates enforce or the docs rely on:

- **No clippy warnings.** CI runs, and `--workspace` is load-bearing — without
  it a warning in `crates/nuvem-domain` goes unlinted:

  ```bash
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  ```
- **Tests for domain logic live in `crates/nuvem-domain`.** The crate's own
  test binary is what pins its behaviour, so it can be checked on its own.
- **New integration test files must be mounted in `tests/suite.rs`** (or the
  domain crate's `tests/suite.rs`). Auto-discovered per-file test targets are
  deliberately off; the file header there explains why.
- **`ARCHITECTURE.md` is machine-checked.** Its module tree is verified in both
  directions against the source tree by `tests/architecture_layout_test.rs`,
  so moving or adding modules means updating the doc in the same change.
- **Commit messages** follow conventional-commit style
  (`fix(ui): …`, `test(domain): …`), as in the existing history.

## Design decisions

Decisions that shape the architecture, and the evidence behind them, live in
[docs/decisions](docs/decisions). `ARCHITECTURE.md` describes how the code is
arranged today; a decision record says why an alternative was taken or refused,
so a question that was already settled is not re-litigated from a worse premise.

## Reporting bugs

Open a GitHub issue. For anything that looks like a security problem, see
[SECURITY.md](SECURITY.md) instead of filing a public issue.

When reporting a rendering bug, the terminal size and the output of
`nuvem info` make it much easier to reproduce. The built-in demo mode
(`nuvem --demo`, see README) reproduces most UI behaviour without any Google
Cloud project.

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE) that covers the project.
