---
title: Cloud Logging
description: Read, tail, search and save Cloud Logging entries for a project or one resource.
sidebar:
  order: 8
---

The Logging workspace is a top-level workspace, a peer of Home,
Configurations, and Resources. There are three ways into it, and they differ
only in what the query is scoped to:

- **`l`** — the Logging workspace on **all recent project logs**, from
  anywhere in the app. It never looks at your selection.
- **`L`** (shifted) — Cloud Logging scoped to the **one resource the current
  view has on screen**. Available anywhere in the Resources workspace: in a
  resource list and in a resource detail pane.
- **The command palette** — `Open project logs` (aliases `logs`, `logging`) is
  the palette form of `l`; the `Open logs for selected ...` entries are the
  palette form of `L`, one per service, offered only when that service has a
  resource the jump can resolve.

`l` and `L` differ only in case and land in the same workspace, so it is worth
stating the distinction once: `l` widens to the project, `L` narrows to the
selection. The help overlay (`?`) says the same thing on both entries.

`L` covers all eleven services — the selected GCE instance, GKE cluster, Cloud
SQL instance, Cloud Run service, Cloud Storage bucket, secret, Cloud Function, Pub/Sub topic, Pub/Sub subscription, BigQuery dataset, or VPC subnet — and resolves the
target from the view rather than from the raw list cursor. Inside a Cloud Run
revision it scopes to that revision rather than to its service; inside the
Cloud Storage object browser, to the bucket being browsed; and with a detail
pane open it follows the pane, not the list cursor still movable behind it.
When nothing can be resolved — an empty list, or no active gcloud context — the
status line says so and the workspace does not change. A jump that does land,
whether from `L` or from the palette, is recorded in navigation history, so
`Alt-←` returns to the resource you jumped from.

Resource-scoped queries are shown as the exact generated filter string in the
workspace header; the project-wide entry shows an `(all recent project logs)`
placeholder. Two of those queries interpolate the project into the identity
they match on — Cloud SQL log entries are keyed by a composite
`<project>:<instance>` database ID, a secret's by the full resource name
`projects/<project>/secrets/<name>` — so those two jumps refuse (with a status
message) when the active context has no project rather than issuing a filter
that would silently match nothing. Logging reads are read-only: the workspace never
mutates log sinks, metrics, or exclusions.

While the tail is running (the default when a query opens), the workspace
polls for new entries on the same interval as `--poll-interval`, in a single
request at a time; new entries are appended and deduplicated by insert ID,
and the buffer is capped so memory use stays bounded. On a quota/rate-limit
failure, polling backs off for a growing number of ticks before retrying;
other errors show a degraded status while keeping the last-known entries
visible.

In the Logging list, use `j`/`k` or the arrows to move the selection and
`Enter` to open the selected entry's detail (payload and labels), `Enter`,
`Esc`, or `q` to close it. Press `t` to pause or resume the tail, `w` to
toggle line wrapping of long message/payload lines in the entry detail view
(wrapped instead of truncated), and `/` to edit a client-side search over the
loaded entries (`Enter` applies it, `Esc` cancels the draft). Press `y` to
copy the selected entry and `i` to copy its insert ID, both from the list and
from an open entry's detail. While an entry's detail is open, use
`PageUp`/`PageDown` to scroll it.

`r` asks for the current window again and keeps the rows already on screen, so
a page that comes back unchanged leaves the view as it was. When no active
gcloud context resolves there is no project to read from and nothing is
fetched; because the rows stay put, the status line reports the missing context
rather than letting the refused refresh look like one that returned the same
page.

Two axes narrow the window server-side, by changing the `gcloud logging read`
call rather than hiding rows that were already fetched. `s` raises and `S`
lowers the severity floor one rank at a time (`DEBUG`, `INFO`, `NOTICE`,
`WARNING`, `ERROR`, `CRITICAL`, `ALERT`, `EMERGENCY`), and the steps stop at
either end rather than wrapping; the default floor is `all`, which adds no
severity clause at all. The since window is seven buckets addressed directly
by the digits `0`-`6` — `1m`, `5m`, `15m`, `30m`, `1h` (the default), `6h`,
`24h` — and `T` still cycles through them shortest to longest. The header
shows both, as `Severity: <floor>` and `Range: <window> [<digit>]`.

The `/` search runs over the loaded buffer and understands four forms: a bare
term is a case-insensitive substring (the default), `~term` is a fuzzy
subsequence, `/pattern/` is a case-insensitive regex, and a leading `!`
inverts any of them. The header spells the sigils out while nothing is
applied and names the mode a pattern selected once one is; a regex that does
not compile shows no rows and reports the reason. Substring rather than regex
is the default so that a bracketed level such as `[ERROR]` matches the
brackets that were typed instead of being read as a character class;
`ARCHITECTURE.md` records why, and why regex stays an explicit opt-in.

`Ctrl-S` saves the currently visible (that is, filtered) entries to
`log-dumps/logs-<timestamp>.log` beside nuvem's own log file — run `nuvem
info` for that path — one sanitized line per entry, and reports the absolute
path it wrote. With nothing visible it writes no file, and when no log
directory resolves it reports a save failure rather than falling back to the
working directory.

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
