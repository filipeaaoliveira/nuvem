---
title: Reading the screen
description: The header, breadcrumb, status line, footer and status markers every view shares.
sidebar:
  order: 3
---

Every view draws inside the same frame:

| Part | What it shows |
| --- | --- |
| Header (two rows) | The active gcloud configuration and project first. The account, service, region and zone follow when the terminal has room. |
| Breadcrumb | Where you are: the workspace, then the service, then the open resource. |
| Body | The workspace itself: a list, a detail pane, a dashboard. |
| Status line | The last operation, its status marker, and the time of the last refresh. Warnings and errors appear here. |
| Footer | The keys that do something in the current view, most useful first. On a narrow terminal the least useful ones are left out whole; a key is never shown without its label. Press `?` for all of them. |

## Status markers

Each status carries a text marker, so it reads the same with or without
color, including under the `no-color` theme:

| Marker | Meaning |
| --- | --- |
| `[OK]` | Idle, or the last operation succeeded |
| `[LOAD]` | A first load is in flight |
| `[SYNC]` | A refresh is in flight; the rows on screen are the previous ones |
| `[STALE]` | A load or refresh failed; what is on screen may be out of date |
| `[ERR]` | The operation failed |

## Workspaces

| Workspace | Key | What it is for |
| --- | --- | --- |
| Home | `h` | [Context health, inventory and recent operations](../guides/home-overview.md) |
| Configurations | `c` | [Your gcloud configurations](../guides/switch-configurations.md) |
| Resources | `g` | [Every service, list and detail](../guides/browse-resources.md) |
| Logging | `l` | [Cloud Logging entries](../guides/cloud-logging.md) |
| Tunnels | `Ctrl-t` | [Port forwards nuvem started](../guides/tunnels.md) |
| FinOps | `Ctrl-f` | [Cost recommendations and budgets](../guides/finops.md) |

## Terminal size

nuvem needs a terminal at least `60x12` (columns by rows). Below that it shows
a message asking for a larger terminal instead of the interface. Modals that
show a command check that the whole command fits. When it does not, they ask
for a larger terminal and refuse `Enter`, so you never confirm a command you
cannot read.

When the Resources body is at least 100 columns wide, the list and a preview
pane sit side by side. See [Browse resources](../guides/browse-resources.md).

## Keyboard only

nuvem reads only the keyboard. It has no mouse support: it does not capture
the mouse, and clicking in the interface does nothing. Because the mouse is
not captured, your terminal's own text selection and copy still work.

## Help

`?` opens the help overlay for the view you are in. It lists the keys in
groups, with your rebound keys shown as rebound. Scroll it with `j`/`k` or the
arrows, page with `PgUp`/`PgDn`, jump to the top or bottom with `g`/`G`, and
close it with `Esc` or `?`. When your config file has problems, help lists
every one of them under `Config problems`.
