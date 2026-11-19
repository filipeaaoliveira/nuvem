# Changelog

All notable changes to nuvem are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) (pre-1.0: minor bumps may break).

## [Unreleased]

### Added

- A terminal UI for Google Cloud covering Compute Engine, GKE, Cloud Run,
  Cloud Functions, Cloud SQL, Cloud Storage, Secret Manager, persistent disks,
  Pub/Sub, BigQuery, VPC networks, subnets and firewall rules, project IAM,
  Cloud Logging and FinOps recommendations, with a home overview, a command
  palette and jumps between related resources.
- Reads through the official Google Cloud client libraries for Rust, using an
  access token from `gcloud config config-helper` for the selected gcloud
  configuration. The token is held in memory only. BigQuery is read through
  `bq`.
- Start, stop, reset, suspend, resume, restart and failover on single or
  marked rows, through the client libraries; switching configuration or
  project through the `gcloud` CLI. Each change asks for confirmation and
  first shows what it will do: the API call for a lifecycle action, the
  command line for a `gcloud` command.
- Cloud Logging scoped to a resource or the whole project, with a live tail,
  a severity floor and a search grammar.
- Tunnels (IAP, SSH local forward, Cloud SQL Auth Proxy) supervised from a
  workspace of their own.
- `config.toml` with themes (`default`, `no-color`, `high-contrast`),
  hotkeys, favorites, columns and sort, and jump rules. An unrecognised key at
  the root or in `[general]` is reported by name.
- Plugins: your own `gcloud` commands, declared in `plugins.toml` and offered
  against the selected row, behind the same confirmation.
- `--demo`, which runs the whole UI against built-in sample data with no Google
  Cloud account or credentials, and `--read-only`, which refuses every mutating
  path.
- `nuvem info` for environment diagnostics, and distinct exit statuses: `0`
  success, `1` failure, `2` usage error, `3` no terminal, `4` startup failure.
- Text from Google Cloud and plugins is sanitised before it reaches the
  screen or the clipboard: escape sequences, bidirectional overrides and
  zero-width characters are removed, and tokens and keys are redacted from
  diagnostics. Secret Manager payloads are never read.
- Release archives for Linux (x86_64 and arm64, glibc 2.28 or newer) and macOS
  (Apple silicon and Intel), each with a `.sha256` file and a build-provenance
  attestation, and a Homebrew formula: `brew install filipeaaoliveira/tap/nuvem`.
