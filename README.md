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
[Demo mode](docs/guides/demo-mode.md).

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

The documentation lives in `docs/`, one topic per page.

**Getting started**

- [Install](docs/getting-started/install.md): what nuvem needs, and how to install it with Homebrew, from a release archive, or from source.
- [First run](docs/getting-started/first-run.md): start nuvem against your gcloud configuration, or try it offline with demo mode.
- [Reading the screen](docs/getting-started/the-screen.md): the header, breadcrumb, status line, footer and status markers every view shares.

**Guides**

- [Home overview](docs/guides/home-overview.md): what the Home workspace shows: context health, inventory, recent operations and degraded causes.
- [Browse resources](docs/guides/browse-resources.md): open a service, move between the list and the detail pane, sort, and go back through history.
- [Filter a list](docs/guides/filter-a-list.md): narrow a resource list with the filter grammar: regex, fuzzy, label and inverse modes.
- [Switch gcloud configurations](docs/guides/switch-configurations.md): activate, create and edit gcloud configurations, and pick a project, region or zone.
- [Command palette](docs/guides/command-palette.md): run any command by name from any view.
- [Batch actions on marked rows](docs/guides/batch-actions.md): mark several GCE or Cloud SQL instances and run one lifecycle action on all of them.
- [Cloud Logging](docs/guides/cloud-logging.md): read, tail, search and save Cloud Logging entries for a project or one resource.
- [Tunnels](docs/guides/tunnels.md): open IAP, SSH and Cloud SQL Auth Proxy port forwards and manage them from one list.
- [Related-resource jumps](docs/guides/related-resource-jumps.md): jump from a resource to the resources it is attached to.
- [FinOps](docs/guides/finops.md): see cost recommendations and budgets for the active project.
- [Demo mode](docs/guides/demo-mode.md): run the full UI offline against built-in fixtures, with no project and no credentials.
- [Read-only mode](docs/guides/read-only-mode.md): refuse every command that changes a cloud resource.

**Configuration**

- [Config file](docs/configuration/config-file.md): where config.toml lives, how settings are layered, and every key it accepts.
- [Themes and frame colors](docs/configuration/themes.md): pick a built-in theme and recolor the frame for a gcloud configuration or project.
- [Hotkeys](docs/configuration/hotkeys.md): rebind the seven global navigation keys with the [hotkeys] table.
- [Service aliases](docs/configuration/aliases.md): add your own names for services in the [aliases] table.
- [Favorites](docs/configuration/favorites.md): list projects, regions and zones first in the context picker.
- [Columns and sort](docs/configuration/columns-and-sort.md): choose which columns each service shows, in which order, and its default sort.
- [Jump rules](docs/configuration/jump-rules.md): write [[jumps]] rules that decide what the related-resource picker offers.
- [Remembered last view](docs/configuration/remembered-view.md): nuvem reopens the service you last used in each gcloud configuration.
- [Reload configuration](docs/configuration/reload.md): apply config.toml changes without restarting.
- [Config problems](docs/configuration/config-problems.md): what happens when config.toml or plugins.toml has a mistake, and where the warnings appear.

**Plugins**

- [How plugins work](docs/plugins/overview.md): offer your own gcloud commands against the selected row, behind a confirmation.
- [Write a plugin](docs/plugins/write-a-plugin.md): add a gcloud command of your own to a service, step by step.
- [Typed inputs](docs/plugins/typed-inputs.md): ask for values the row cannot supply before a plugin runs.
- [Plugin output](docs/plugins/plugin-output.md): read what a plugin run printed.
- [Plugin file reference](docs/plugins/plugin-file-reference.md): every key plugins.toml accepts, every input type, and the template fields each service offers.

**Reference**

- [Keys](docs/reference/keys.md): every key nuvem binds, view by view, with the label the footer shows.
- [Command line](docs/reference/cli.md): every flag and subcommand, the exit codes, the files nuvem uses and the environment it reads.
- [Palette commands](docs/reference/commands.md): every command the command palette offers, its search aliases, and when it appears.

**Services**

- [All services](docs/reference/services/overview.md): every service nuvem can browse, the names it answers to, and what you can do in each.
- [GCE instances](docs/reference/services/gce-instances.md): browse Compute Engine instances, run lifecycle actions, SSH, and copy their fields.
- [GKE clusters](docs/reference/services/gke-clusters.md): browse GKE clusters, read their detail, and open one in k9s.
- [Cloud SQL](docs/reference/services/cloud-sql.md): browse Cloud SQL instances and start, stop, restart or fail them over.
- [Cloud Run](docs/reference/services/cloud-run.md): browse Cloud Run services, their traffic split and their revisions.
- [Cloud Storage](docs/reference/services/cloud-storage.md): browse buckets and objects, read metadata, and copy gs:// URIs.
- [Secret Manager](docs/reference/services/secret-manager.md): browse secret metadata and versions. Payloads are never read.
- [Cloud Functions](docs/reference/services/cloud-functions.md): list 1st and 2nd gen functions across every region.
- [Persistent disks](docs/reference/services/persistent-disks.md): list zonal and regional persistent disks and find the ones nothing uses.
- [Pub/Sub topics](docs/reference/services/pubsub-topics.md): list Pub/Sub topics with their subscription count, retention and encryption key.
- [Pub/Sub subscriptions](docs/reference/services/pubsub-subscriptions.md): list every Pub/Sub subscription in the project with its topic and delivery settings.
- [BigQuery datasets](docs/reference/services/bigquery-datasets.md): browse datasets, the tables inside them, and each table's column schema.
- [Service accounts](docs/reference/services/service-accounts.md): list the project's IAM service accounts.
- [IAM policy](docs/reference/services/iam-policy.md): read the project IAM policy as one row per role and member.
- [VPC networks](docs/reference/services/vpc-networks.md): list VPC networks with their subnet mode, routing mode and subnet count.
- [VPC subnets](docs/reference/services/vpc-subnets.md): list every subnet in every region in one table.
- [Firewall rules](docs/reference/services/firewall-rules.md): list firewall rules with their action, targets and source or destination ranges.

**Troubleshooting**

- [Common problems](docs/troubleshooting/common-problems.md): what nuvem's error messages mean and how to fix the problems people meet most.
- [Questions](docs/troubleshooting/faq.md): short answers about credentials, platforms, safety and why BigQuery needs bq.

Contributors: [ARCHITECTURE.md](ARCHITECTURE.md) explains how the code is put together.

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
