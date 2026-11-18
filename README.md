# nuvem

[![CI](https://github.com/filipeaaoliveira/nuvem/actions/workflows/ci.yml/badge.svg)](https://github.com/filipeaaoliveira/nuvem/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

(Portuguese for "cloud".)

A terminal UI for Google Cloud, in the spirit of k9s: browse and operate
Compute Engine, GKE, Cloud Run, Cloud Functions, Cloud SQL, Cloud Storage,
Secret Manager, Pub/Sub, BigQuery, VPC, IAM, Cloud Logging and FinOps
recommendations from one keyboard-driven interface.

nuvem reads Google Cloud through the official Google Cloud client libraries
for Rust, authenticated by a short-lived access token that
`gcloud config config-helper` issues for the gcloud configuration you selected.
The token is held in memory for its lifetime and never written to disk, so
nuvem stores no credentials of its own. Everything that changes state —
activating a configuration, lifecycle actions, tunnels, SSH and plugins — shells
out to the `gcloud` CLI you already have, using your native gcloud
configurations, and asks for confirmation — showing the exact command — before
anything that mutates. BigQuery browsing shells out to `bq`. Output surfaced in
diagnostics is scrubbed of tokens and keys before it can be displayed or
copied. The reasoning behind the split is recorded in
`docs/decisions/0001-native-sdk-for-read-paths.md`.

## Requirements

- The [Google Cloud CLI](https://cloud.google.com/sdk/docs/install) (`gcloud`)
  installed and authenticated (`gcloud auth login`).
- A terminal. Unix-like platforms (Linux, macOS) are what CI covers; process
  control uses POSIX APIs, so Windows is currently unsupported outside WSL.
- Outbound HTTPS to `*.googleapis.com`. Reads are direct API calls; only
  `gcloud`'s own invocations honour gcloud's `proxy/*` properties.
- To build from source: a C toolchain and `cmake` on `PATH`. The TLS stack
  behind the Google Cloud client libraries compiles native code at build time.

## Install

### Download a release

Each [GitHub release](https://github.com/filipeaaoliveira/nuvem/releases)
carries a `nuvem-<target>.tar.gz` archive with a `.sha256` sidecar:

```bash
shasum -a 256 -c nuvem-<target>.tar.gz.sha256
tar xzf nuvem-<target>.tar.gz && install nuvem-<target>/nuvem ~/.local/bin/
```

| Platform | Target |
| --- | --- |
| Linux, x86_64 | `x86_64-unknown-linux-gnu` |
| Linux, arm64 | `aarch64-unknown-linux-gnu` |
| macOS, Apple silicon | `aarch64-apple-darwin` |
| macOS, Intel | `x86_64-apple-darwin` |

CI builds each one natively on a runner of that platform, runs the test suite
there and smoke-tests the binary it packages.

The Linux binaries are linked against glibc 2.28, so they run on Pop!_OS 22.04,
Ubuntu 20.04, Debian 10 and RHEL 8 and newer. Check with `ldd --version` if
you are unsure; a binary that is too new for its glibc fails at startup with
`GLIBC_2.xx not found` rather than at install time.

Every archive also carries a signed build-provenance attestation, which
records the workflow, commit and run that produced the file — a checksum shows
only that a download was not corrupted, while this shows where it came from:

```bash
gh attestation verify nuvem-x86_64-unknown-linux-gnu.tar.gz --repo filipeaaoliveira/nuvem
```

nuvem is not on crates.io yet, so install it from a release or build it from
source.

### Build from source

Build with a recent stable Rust toolchain (`rustup` picks up the pinned
version automatically):

```bash
git clone https://github.com/filipeaaoliveira/nuvem
cd nuvem
cargo install --path . --locked
```

## Quick start

```bash
nuvem            # run against your active gcloud configuration
nuvem --demo     # try the full UI offline, no project or credentials needed
nuvem --read-only
nuvem info       # print environment diagnostics
```

Press `?` in the app for context-sensitive help, `Ctrl-C` to quit.

`nuvem --demo` is the fastest way to see it: it runs the whole UI against
embedded fixtures, with no Google Cloud project and no credentials, and every
mutating command is a no-op. See
[docs/demo-and-read-only.md](docs/demo-and-read-only.md).

## What you get

- **One list, every service.** Compute Engine, GKE, Cloud Run, Cloud Functions,
  Cloud SQL, Cloud Storage, Secret Manager, persistent disks, Pub/Sub,
  BigQuery, VPC networks/subnets/firewall rules and project IAM — filtered,
  sorted and column-configurable, with a detail pane per row.
- **Your gcloud configurations are the context.** Switch configuration or
  project from inside the app; the rows you were looking at come back rather
  than being re-fetched from nothing.
- **Operations, with the argv in front of you.** Start, stop, reset, suspend
  and failover — single-row or across marked rows — each behind a confirmation
  that shows the command that will run.
- **Cloud Logging**, scoped to the resource you are on, with a live tail, a
  severity floor and a search grammar.
- **Tunnels**: IAP, SSH local-forward and the Cloud SQL Auth Proxy, supervised
  and killable from a workspace of their own.
- **Jumps** between related resources, and **plugins** — your own `gcloud`
  commands, offered against the selected row.
- **`--read-only`**, which refuses every mutating path rather than hiding it.

## Documentation

| | |
| --- | --- |
| [docs/keys.md](docs/keys.md) | Every key, in one place |
| [docs/configuration.md](docs/configuration.md) | Config file: schema, themes, hotkeys, columns, favorites, jump rules |
| [docs/plugins.md](docs/plugins.md) | Declaring your own `gcloud` commands |
| [docs/features.md](docs/features.md) | What each workspace does, service by service |
| [docs/demo-and-read-only.md](docs/demo-and-read-only.md) | Running without a project, and without write access |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How the code is put together |

## Exit status

`nuvem info` and `--demo` are meant to be runnable from a script, so failures
are told apart rather than all reported as `1`:

| Code | Meaning |
| ---- | ------- |
| `0`  | Success |
| `1`  | The session ran and ended in failure |
| `2`  | A bad flag or unknown subcommand |
| `3`  | stdin or stdout is not a terminal — no TUI to draw |
| `4`  | Could not start: the log file could not be created or opened |

Code `3` is the one worth branching on in CI: it means nuvem was run without a
TTY, not that anything broke.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup and the
quality gates a change must pass. Security reports go through
[SECURITY.md](SECURITY.md), not public issues.

## License

nuvem is licensed under the [MIT License](LICENSE).

nuvem is an independent project. It is not affiliated with, endorsed by or
sponsored by Google. Google Cloud and gcloud are trademarks of Google LLC.
