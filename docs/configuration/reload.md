---
title: Reload configuration
description: Apply config.toml changes without restarting.
sidebar:
  order: 9
---

## Reload

1. Edit `config.toml` and save it.
2. Press `:` and run **Reload configuration** (aliases `reload config`,
   `config reload`).
3. Read the status line. It says `configuration reloaded`, or quotes the one
   warning, or counts the problems and names the key that opens the list.

Reload re-reads the config file. It never touches cloud state and never
refetches a list.

## What reload applies

| Setting | Applied by reload |
| --- | --- |
| `general.theme` | yes |
| `[theme.overrides]`, `[theme.project_overrides]` | yes |
| `general.sidebar_visible` | yes |
| `general.poll_interval_secs` | yes |
| `general.read_only` | only to turn read-only on; reload never turns it off |
| `[aliases]` | yes |
| `[hotkeys]` | yes |
| `[favorites]` | yes |
| `[columns.<service>]` | yes; see [Columns and sort](columns-and-sort.md) |
| `[[jumps]]` | yes; see [Jump rules](jump-rules.md) |
| `general.default_service` | no: restart required |
| the starting configuration (`--configuration`) | no: restart required |
| `plugins.toml` | no: restart required |

When a setting that needs a restart has changed, the status line ends with
`restart required for:` and names it, for example
`restart required for: plugins`.

Command-line flags keep the values nuvem started with: reload reads the same
flags again.

## When the file is broken

A file that cannot be read or parsed, or that declares an unknown `version`,
leaves the running configuration untouched and shows the error in the status
line. A problem inside one table drops only what that table's rules drop; see
[Config problems](config-problems.md).
