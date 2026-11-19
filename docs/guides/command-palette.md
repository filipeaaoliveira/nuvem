---
title: Command palette
description: Run any command by name from any view.
sidebar:
  order: 6
---

The command palette is a global command mode. Press `:` in any normal view to
open it. It lists:

- Navigation commands: Home, Configurations, Tunnels, FinOps, Logging
  (`Open project logs`), one command per service, Refresh, Reload
  configuration, Help, and Switch project / context.
- The actions valid for the current selection.
- The [plugins](../plugins/overview.md) that apply to the current selection.

Type to search. A command matches on its label or on one of its aliases, such
as `gce`, `compute`, `gke`, `clusters`, `sql`, `run`, `gcs`, `logs`, `ssh`,
`tunnels` or `k9s`. It also matches on your own
[`[aliases]`](../configuration/aliases.md). An exact label or alias ranks
first, then a prefix, then any other substring. If a search matches nothing,
the palette shows your query and suggests a few commands to try. The palette
never opens empty.

## Run a command

1. Press `:`.
2. Type part of the command's name or an alias, for example `sql` or `logs`.
   `↑`/`↓` move through the matches. `j` and `k` type into the search.
3. Press `Enter` to run the selected command, or `Esc` to close the palette.

A command that changes something first opens a confirmation, and nothing runs
until you confirm. The confirmation shows the API call for a lifecycle action,
or the command line for anything that runs `gcloud`. See
[Before a change runs](../reference/services/overview.md#before-a-change-runs).

[Palette commands](../reference/commands.md) lists every command, its aliases
and when it is offered.
