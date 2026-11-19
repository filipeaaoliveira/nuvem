---
title: Command line
description: Every flag and subcommand, the exit codes, the files nuvem uses and the environment it reads.
sidebar:
  order: 2
---

```text
nuvem [OPTIONS] [COMMAND]
```

With no command, nuvem starts the interface. Flags override the
[config file](../configuration/config-file.md), which overrides the built-in
defaults.

## Flags

| Flag | Default | What it does |
| --- | --- | --- |
| `--configuration <NAME>` | the active gcloud configuration | Start on this gcloud configuration. nuvem asks gcloud to activate it, then reloads the configuration list. |
| `--poll-interval <SECONDS>` | `5`, or `general.poll_interval_secs` | Seconds between background refreshes of the active service and of the Logging tail. `0` turns background refresh off. |
| `--config <PATH>` | `~/.config/nuvem/config.toml` on Linux, `~/Library/Application Support/nuvem/config.toml` on macOS | Read the config file from this path. `state.toml` is kept beside it. |
| `--plugins <PATH>` | `plugins.toml` in the same directory as the default config file | Read plugins from this path. Never derived from `--config`. |
| `--demo` | off | Serve built-in fixtures instead of calling Google Cloud. See [Demo mode](../guides/demo-mode.md). |
| `--read-only` | off, or `general.read_only` | Refuse every command that changes a cloud resource. See [Read-only mode](../guides/read-only-mode.md). |
| `--batch-concurrency <N>` | `4` | How many actions a [batch](../guides/batch-actions.md) runs at once. |
| `--log-file <PATH>` | `~/.cache/nuvem/nuvem.log` on Linux, `~/Library/Caches/nuvem/nuvem.log` on macOS | Where nuvem writes its own log. `Ctrl-S` in Logging saves beside it. |
| `--gcloud-binary <PATH>` | `gcloud` | The gcloud CLI to run. |
| `--bq-binary <PATH>` | `bq` | The `bq` CLI that BigQuery browsing runs. |
| `--k9s-binary <PATH>` | `k9s` | The k9s binary **Open selected GKE cluster in k9s** launches. |
| `--kubectl-binary <PATH>` | `kubectl` | The kubectl binary nuvem asks whether a GKE kube context already exists. |
| `--cloud-sql-proxy-binary <PATH>` | `cloud-sql-proxy` | The Cloud SQL Auth Proxy that Cloud SQL tunnels run. |
| `-h`, `--help` | | Print help and exit. |
| `-V`, `--version` | | Print the version and exit. |

A binary path without a `/` is looked up on your `PATH`. Binary paths,
`--batch-concurrency`, `--log-file` and `--configuration` have no config-file
key.

`--config`, `--plugins`, `--demo` and `--read-only` can also follow a
subcommand, as in `nuvem info --config ./dev.toml`.

## Commands

| Command | What it does |
| --- | --- |
| `info` | Print the config, plugin, log and cache paths, the active theme and any config warning, then exit. It does not start the interface and needs no terminal. |
| `help` | Print help, or the help of a subcommand, and exit. |

`nuvem info` prints:

```text
nuvem 0.1.0
config:  /home/dev/.config/nuvem/config.toml (loaded)
plugins: /home/dev/.config/nuvem/plugins.toml (not found)
log:     /home/dev/.cache/nuvem/nuvem.log
cache:   /home/dev/.cache/nuvem
theme:   default
```

`config:` ends in `loaded`, `not found` or `error: <message>`. `plugins:` ends
in `not found` or `<n> loaded`. Each config warning follows on its own
`warning:` line.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | The session ran and ended in failure, or failed for a reason with no code of its own |
| `2` | A bad flag or an unknown subcommand |
| `3` | stdin or stdout is not a terminal, so there is no interface to draw |
| `4` | nuvem could not start: the log file could not be created or opened |

Code `3` is the one to branch on in a script or CI job: it means nuvem ran
without a terminal, not that anything broke. These codes do not change between
releases. A new kind of failure gets a new code.

## Files

| File | Default location | Written by nuvem |
| --- | --- | --- |
| `config.toml` | the platform config directory, `nuvem/` | never |
| `plugins.toml` | the platform config directory, `nuvem/` | never |
| `state.toml` | beside the config file in use | yes: the last service per gcloud configuration. See [Remembered last view](../configuration/remembered-view.md). |
| `nuvem.log` | the platform cache directory, `nuvem/` | yes, readable only by you |
| `log-dumps/logs-<timestamp>.log` | beside `nuvem.log` | yes, when you press `Ctrl-S` in Logging |

## Environment

| Variable | Effect |
| --- | --- |
| `RUST_LOG` | Sets what nuvem writes to its log file, for example `RUST_LOG=debug`. The default is `info`. The Google Cloud client libraries and the HTTP stack stay at `info` whatever you set, so request payloads and tokens never reach the log. |
| `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` | On Linux, move the config and cache directories. |
| `TMUX` | Recorded once at startup in nuvem's log, at `debug` level. It changes nothing else. |

Every command nuvem runs (`gcloud`, `bq`, `k9s`, `kubectl`,
`cloud-sql-proxy`, and plugins) inherits your environment. Reads through the
Google Cloud client libraries do not use gcloud's `proxy/*` properties. They
need direct HTTPS access to `*.googleapis.com`.
