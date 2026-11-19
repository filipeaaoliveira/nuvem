---
title: Cloud Logging
description: Read, tail, search and save Cloud Logging entries for a project or one resource.
sidebar:
  order: 8
---

The Logging workspace is a top-level workspace, like Home, Configurations and
Resources. It reads Cloud Logging entries for the whole project or for one
resource.

## Open logs

There are three ways into the workspace. They differ only in what the query
is scoped to:

- `l` opens the Logging workspace on all recent project logs, from anywhere in
  the app. It never looks at your selection.
- `L` (shifted) opens Cloud Logging scoped to the one resource the current
  view has on screen. It works anywhere in the Resources workspace: in a
  resource list and in a resource detail pane.
- In the command palette, `Open project logs` (aliases `logs`, `logging`) is
  the palette form of `l`. The `Open logs for selected ...` entries are the
  palette form of `L`, one per service. Each one is offered only when that
  service has a resource the jump can resolve.

`l` widens to the project, and `L` narrows to the selection. The help overlay
(`?`) states this difference on both entries.

`L` covers all eleven services: the selected GCE instance, GKE cluster, Cloud
SQL instance, Cloud Run service, Cloud Storage bucket, secret, Cloud Function,
Pub/Sub topic, Pub/Sub subscription, BigQuery dataset, or VPC subnet. It
resolves the target from the view rather than from the raw list cursor.
Inside a Cloud Run revision, it scopes to that revision, not to its service. Inside the Cloud Storage object browser, it scopes to the bucket you
are browsing. With a detail pane open, it follows the pane, not the list
cursor that can still move behind it. If nothing resolves, for example because
the list is empty or no gcloud context is active, the status line says so and
the workspace does not change. A jump that lands, from `L` or from the
palette, is recorded in navigation history, so `Alt-←` returns to the resource
you jumped from.

For a resource-scoped query, the workspace header shows the exact filter
string that nuvem built. For the project-wide query, it shows
`(all recent project logs)`. Two of these filters include the project in the
identity they match on. Cloud SQL log entries are keyed by a composite
`<project>:<instance>` database ID, and a secret's entries by the full
resource name `projects/<project>/secrets/<name>`. If the active context has
no project, these two jumps refuse with a status message instead of sending a
filter that matches nothing.

The workspace only reads logs. It never changes log sinks, metrics, or
exclusions.

## Follow the tail

The tail runs by default when a query opens. While it runs, the workspace
polls for new entries on the same interval as `--poll-interval`, one request
at a time. New entries are appended and deduplicated by insert ID. The buffer has a
cap, so memory use stays bounded. If a request fails on a quota or rate limit,
polling waits a growing number of ticks before it retries. Other errors show a
degraded status and keep the last entries on screen.

`r` asks for the current window again and keeps the rows already on screen. If
the page comes back unchanged, the view stays as it was. If no gcloud context
is active, there is no project to read from, and nuvem fetches nothing. The
rows stay on screen, so the status line reports the missing context. That
message tells a refused refresh apart from one that returned the same page.

## Read the entries

In the Logging list:

- `j`/`k` or the arrows move the selection.
- `Enter` opens the selected entry's detail, with its payload and labels.
  `Enter`, `Esc`, or `q` closes it.
- `t` pauses or resumes the tail.
- `w` toggles line wrapping of long message and payload lines in the entry
  detail view. With wrapping on, long lines wrap instead of being truncated.
- `/` edits a client-side search over the loaded entries. `Enter` applies it,
  and `Esc` cancels the draft.
- `y` copies the selected entry, and `i` copies its insert ID. Both work from
  the list and from an open entry's detail.
- `PageUp`/`PageDown` scroll an open entry's detail.

## Narrow the window

Two settings narrow the window on the server. They change the query that
nuvem sends to Cloud Logging, instead of hiding rows it already fetched.

- `s` raises and `S` lowers the severity floor one rank at a time (`DEBUG`,
  `INFO`, `NOTICE`, `WARNING`, `ERROR`, `CRITICAL`, `ALERT`, `EMERGENCY`). The
  steps stop at either end and do not wrap. The default floor is `all`, which
  adds no severity clause.
- The since window has seven buckets, and the digits `0`-`6` pick one
  directly: `1m`, `5m`, `15m`, `30m`, `1h` (the default), `6h`, `24h`. `T`
  still cycles through them, shortest to longest.

The header shows both, as `Severity: <floor>` and `Range: <window> [<digit>]`.

## Search the loaded entries

The `/` search runs over the loaded buffer and understands four forms:

- A bare term is a case-insensitive substring. This is the default.
- `~term` is a fuzzy subsequence.
- `/pattern/` is a case-insensitive regex.
- A leading `!` inverts any of them.

While no search is applied, the header spells out the sigils. Once one is
applied, the header names the mode the pattern selected. A regex that does
not compile shows no rows and reports the reason. Substring is the default so
that a bracketed level such as `[ERROR]` matches the brackets you typed. As a
regex, `[ERROR]` is a character class. `ARCHITECTURE.md` records why, and why
regex stays an explicit opt-in.

## Save entries to a file

`Ctrl-S` saves the visible entries, after the search filter, to
`log-dumps/logs-<timestamp>.log`. That directory sits beside nuvem's own log
file, and `nuvem info` prints its path. Each entry becomes one sanitized line,
and nuvem reports the absolute path it wrote. If no entries are visible, it
writes no file. If no log directory resolves, it reports a save failure and
does not fall back to the working directory.

## Severity floor and time window

The severity floor, lowest to highest. `s` moves one step up, `S` one step
down.

<!-- log-severities -->

| Floor | Shows |
| --- | --- |
| `all` | every entry; no severity clause is sent (the default) |
| `DEBUG` | `DEBUG` and above |
| `INFO` | `INFO` and above |
| `NOTICE` | `NOTICE` and above |
| `WARNING` | `WARNING` and above |
| `ERROR` | `ERROR` and above |
| `CRITICAL` | `CRITICAL` and above |
| `ALERT` | `ALERT` and above |
| `EMERGENCY` | `EMERGENCY` only |

The time window. Press the digit to pick a window, or `T` to cycle.

<!-- log-windows -->

| Key | Window |
| --- | --- |
| `0` | `1m` |
| `1` | `5m` |
| `2` | `15m` |
| `3` | `30m` |
| `4` | `1h` (the default) |
| `5` | `6h` |
| `6` | `24h` |
