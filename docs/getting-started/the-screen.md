---
title: Reading the screen
description: The header, breadcrumb, status line, footer and status markers every view shares.
sidebar:
  order: 3
---

nuvem has six workspaces: Home, Configurations, Resources, Logging, Tunnels
and FinOps. A workspace is a top-level screen that one key opens, listed under
[Workspaces](#workspaces). What a workspace shows at a given moment, such as a
list, a detail pane or a dashboard, is a view. Every view draws inside the
same frame:

| Part | What it shows |
| --- | --- |
| Header (two rows) | The active gcloud configuration and project first. The account, service, region and zone follow when the terminal has room. |
| Breadcrumb | Where you are: the workspace, then the service, then the open resource. |
| Body | The workspace itself: a list, a detail pane, a dashboard. |
| Status line | A status marker and the time of the last refresh. While something loads, or after a warning or an error, it names that operation. Otherwise it names the view on screen. |
| Footer | The keys that do something in the current view, most useful first. On a narrow terminal, the least useful keys are left out whole, and a key never shows without its label. Press `?` for all of them. |

The configuration, account, project, region and zone in the header are the
context. Switching gcloud configuration, or editing the active
configuration's project, region or zone, changes the context.

Above a resource list, the tools line shows the applied filter, the row
count, the sort and a few key hints, such as `w wide`.

A view is a normal view while no confirmation, command palette, help overlay
or text field has the keyboard. Most keys work only in a normal view.

## Status markers

Each status carries a text marker, so it reads the same with or without
color, including under the `no-color` theme:

| Marker | Meaning |
| --- | --- |
| `[OK]` | Idle, or the last operation succeeded |
| `[LOAD]` | A first load is in flight |
| `[SYNC]` | A refresh is in flight. The rows on screen are still the previous ones |
| `[STALE]` | A load or refresh failed. What is on screen can be out of date |
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

nuvem needs a terminal at least `60x12` (columns by rows). In a smaller
terminal, it shows a message that asks for a larger one instead of the
interface. A confirmation or preview that shows a command checks that the
whole command fits. If it does not fit, the confirmation asks for a larger
terminal and refuses `Enter`, so you never confirm a command you cannot read.

When the Resources body is at least 100 columns wide, the list and a preview
pane sit side by side. See [Browse resources](../guides/browse-resources.md).

## Keyboard only

nuvem reads only the keyboard. It does not capture the mouse, and clicking in
the interface does nothing. Because nuvem leaves the mouse alone, your
terminal's own text selection and copy still work.

## Help

`?` opens the help overlay for the view you are in. It covers everything below
the header, so the configuration and project stay in view. It lists the keys in
groups. A key you rebound appears as the key you chose. Scroll it with `j`/`k` or the
arrows, page with `PgUp`/`PgDn`, jump to the top or bottom with `g`/`G`, and
close it with `Esc` or `?`. If your config file has problems, help lists
every one of them under `Config problems`.
