---
title: Command palette
description: Run any command by name from any view.
sidebar:
  order: 6
---

Press `:` from any normal view to open the palette as a global command mode. It lists
navigation commands — Home, Configurations, Tunnels, FinOps, Logging (`Open
project logs`), one command per service, Refresh, Reload configuration, Help,
and Switch project / context — alongside the actions valid for the current
selection and the [plugins](../plugins/overview.md) that apply to it.

Type to search. A command matches on its label or one of its aliases, such as
`gce`, `compute`, `gke`, `clusters`, `sql`, `run`, `gcs`, `logs`, `ssh`,
`tunnels` or `k9s`, and on your own [`[aliases]`](../configuration/aliases.md).
Matches are ranked: an exact label or alias first, then a prefix, then any
substring. When a search matches nothing the palette shows the query and a
recovery hint; opening it never starts empty.

## Run a command

1. Press `:`.
2. Type part of the command's name or an alias, for example `sql` or `logs`.
   `↑`/`↓` move through the matches; `j` and `k` type into the search.
3. Press `Enter` to run the selected command, or `Esc` to close the palette.

A command that changes something opens a confirmation showing what it is about
to do before anything runs: the API call for a lifecycle action, the command
line for anything that runs `gcloud`. See
[Before a change runs](../reference/services/overview.md#before-a-change-runs).

[Palette commands](../reference/commands.md) lists every command, its aliases
and when it is offered.
