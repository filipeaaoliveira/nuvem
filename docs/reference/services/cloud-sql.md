---
title: Cloud SQL
description: Browse Cloud SQL instances and start, stop, restart or fail them over.
sidebar:
  order: 4
---

For Cloud SQL instances, pressing `r` runs:

```bash
gcloud --configuration <active-config> sql instances list --format=json --limit=500
```

Select an instance and press `Enter` to load a read-only detail view. The detail
command is:

```bash
gcloud --configuration <active-config> sql instances describe <name> --format=json
```

Use `[` and `]` to move across Summary, Connectivity, Maintenance, Backups,
Replicas, Users, and JSON tabs. The Users list is a second `gcloud sql users
list` call, lazy-loaded the first time the Users tab is opened. Passwords and
certificates are redacted at parse time and never appear on any tab, including
the raw JSON one.

In the Cloud SQL workspace, select an instance and press `:` to open the command
palette. Mutation availability follows the instance status: stopped instances
can be started; runnable instances can be stopped or restarted; and Failover is
offered only for runnable HA/REGIONAL instances. Every mutation opens the shared
confirmation modal showing the exact gcloud command before it runs — Start/Stop
via `sql instances patch --activation-policy=ALWAYS|NEVER`, Restart and Failover
via the `sql instances restart`/`failover` verbs. Press `Enter` to confirm or
`Esc` to cancel. On success the app refreshes the instance list as with GCE.
The same four mutations also run against a marked set, in which case the
palette offers a command when any marked instance is eligible for it — see
[Batch actions on marked rows](../../guides/batch-actions.md).
