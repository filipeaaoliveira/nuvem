# Changelog

All notable changes to nuvem are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) (pre-1.0: minor bumps may break).

## [Unreleased]

### Changed

- The project is renamed from gtest to nuvem. The binary is `nuvem`, and the
  configuration, plugins, state and log files moved from `gtest/` to `nuvem/`
  under the platform config and cache directories. Move an existing
  `config.toml` and `plugins.toml` across by hand; the old directory is not
  read.
- Release binaries for Linux are linked against glibc 2.28 instead of the
  build image's 2.39, so they run on Pop!_OS 22.04, Ubuntu 20.04, Debian 10
  and RHEL 8 rather than requiring Ubuntu 24.04 or newer. The release build
  reads the requirement back out of the binary and refuses to publish one
  that needs a newer glibc than the README promises.

- Reads go through the official Google Cloud Rust client libraries instead of
  a `gcloud` subprocess: every list and every detail load, except BigQuery,
  which still uses `bq`. Authentication comes from `gcloud config config-helper`
  for the active configuration and is cached in memory to the token's expiry,
  so `gcloud` is still required. Configuration management, lifecycle actions,
  tunnels, SSH and plugins are unchanged. Building from source now needs a C
  toolchain and `cmake`. See `docs/decisions/0001-native-sdk-for-read-paths.md`.
- Server-side `--filter` pushdown is gone on the native read paths; regex,
  fuzzy and inverse modes narrow client-side as before, and a label-mode read
  (`-l`) is routed through `gcloud`, the one backend that can honour it.
- The Cloud Storage object browser pages with the token the service issues
  rather than re-deriving folders from a flat listing, which was wrong on large
  buckets, not merely slow.
- The log file holds every client library at `info` regardless of `RUST_LOG`
  and is created owner-only.

### Added

- Open-source scaffolding: LICENSE (MIT), CONTRIBUTING, SECURITY policy,
  code of conduct, issue/PR templates, Dependabot, and a cargo-deny CI gate.

### Fixed

- Resource table cells are sanitised where they are built, instead of relying
  on ratatui dropping unprintable graphemes as it writes its buffer. Escape
  sequences and reordering characters in a resource name could previously
  reach a cell intact.

- Characters that reorder or hide text — `U+202E` and the other bidirectional
  overrides and isolates, and zero-width separators like `U+200B` — no longer
  survive into a rendered surface or the clipboard. A resource name carrying
  one could previously render, and be pasted, as a different name. Joiners
  (`U+200C`/`U+200D`) are deliberately unaffected: they are ordinary text.

- A plugin printing ANSI or OSC escape sequences left readable residue on
  screen and on the clipboard (`ESC[31m` arrived as `[31m`): redaction removed
  the introducing control character before the sequence could be recognised.
  Whole sequences are now stripped at capture, before redaction.

### Changed

- An unrecognised setting in `config.toml` — at the root or in `[general]` —
  is now reported as a warning naming the key, instead of being silently
  ignored. `poll_interval_sec` (one character off `poll_interval_secs`) used
  to load clean and do nothing. The rest of the file still applies.

- Exit statuses are now distinguished rather than all reported as `1`: `2` for
  a usage error, `3` for "no terminal attached", `4` for a startup failure.
  Documented in the README; `1` still covers every uncategorised failure, so
  existing `!= 0` checks are unaffected.

- `plugins.toml` is refused when world-writable, and all three configuration
  files are refused when they are not ordinary files or exceed 1 MiB. A named
  pipe at one of these paths previously hung startup indefinitely.
- Launches whose interpolated values would be read as options by the child
  (a resource named `--zone`, say) are refused at the point they are offered,
  rather than reaching `gcloud`'s argv.

### Initial scope

The first version: terminal UI over the `gcloud` CLI covering
Compute Engine, GKE, Cloud Run, Cloud Functions, Cloud SQL, Cloud Storage,
Secret Manager, persistent disks, Pub/Sub, BigQuery, VPC (networks, subnets,
firewalls), IAM, Cloud Logging, FinOps recommendations, tunnels, plugins,
themes, and a fixture-backed demo mode.
