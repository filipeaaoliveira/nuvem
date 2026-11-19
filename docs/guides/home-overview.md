---
title: Home overview
description: "What the Home workspace shows: context health, inventory, recent operations and degraded causes."
sidebar:
  order: 1
---

The Home workspace (`h`) is a read-only dashboard over the state nuvem has
already loaded. It never starts a resource load on its own.

## Context health

Context health shows whether `gcloud` is available and authenticated, the
active account, and any problem the last `gcloud` check found, named by its
category. That check covers the `gcloud` CLI only. A read through the client libraries
can fail on its own, for example with an expired token, a disabled API, or no
route to `googleapis.com`. That failure shows on the service's row, not here.

Press `r` on Home to run the `gcloud` check again. It does not reload the
resource inventory. To refresh a service's rows, switch to the Resources
workspace and press `r` there.

## Inventory by status

The inventory has one line per service (GCE instances, GKE clusters, Cloud
SQL, Cloud Run, Cloud Storage, Secret Manager, Cloud Functions, Persistent
disks, Pub/Sub topics, Pub/Sub subscriptions, BigQuery datasets, Service
accounts, IAM policy, VPC networks, VPC subnets, Firewall rules). That is
every service nuvem can browse. Services that have a lifecycle status show
status counts. Each line also shows how fresh its data is:

| Marker | Meaning |
| --- | --- |
| `Not loaded` | Never fetched yet |
| `Loaded (empty)` | Fetched, zero rows |
| `<n> loaded` | Fetched, has rows |
| `Stale` | Fetched, but out of date for the active context |

If the terminal is too short to show every line, the block ends with
`+N more`, so no skipped service goes unmentioned. On a terminal at least 96
columns wide, a Count trend column appears at the right edge. It draws each
service's row count over its recent refreshes as a small bar chart, scaled to
that service's own peak. A service appears there once it has been counted at
least twice.

## Recent operations

Recent operations lists the latest operations you ran in nuvem, with their
outcome (pending, succeeded, failed, cancelled, unknown) and detail. These
are GCE and Cloud SQL lifecycle actions, and plugin runs. `cancelled`
means the command was never sent. `unknown` means it was sent but nuvem never
saw its result, so refresh the resource to see its real state.

## Degraded causes

When context health or a service line reports a problem, Home shows a short
headline instead of the raw error output. The headline is sanitized, names a
classified reason, and adds a hint for what to do, for example
`permission denied — Check IAM roles for this account and project`. To copy
the sanitized technical detail of the last diagnostic to the clipboard, press
`Y` in any normal view.
