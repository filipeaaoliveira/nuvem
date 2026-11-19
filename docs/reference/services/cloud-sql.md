---
title: Cloud SQL
description: Browse Cloud SQL instances and start, stop, restart or fail them over.
sidebar:
  order: 4
---

For Cloud SQL instances, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> sql instances list --format=json --limit=500
```

To open a read-only detail view, select an instance and press `Enter`. The
equivalent detail command is:

```bash
gcloud --configuration <active-config> sql instances describe <name> --format=json
```

Use `[` and `]` to move across Summary, Connectivity, Maintenance, Backups,
Replicas, Users, and JSON tabs. The Users list is a second call (the
equivalent of `gcloud sql users list`), loaded the first time you open the
Users tab. Passwords and certificates are redacted at parse time. They never
appear on any tab, including the raw JSON one.

In the Cloud SQL workspace, select an instance and press `:` to open the command
palette. The actions on offer follow the instance status:

- A stopped instance can be started.
- A runnable instance can be stopped or restarted.
- Failover is offered only for a runnable HA/REGIONAL instance.

Every action opens the shared confirmation, which shows the API call it will
make. Start and Stop show `sql.instances.patch` with `activationPolicy=ALWAYS`
or `NEVER`. Restart and Fail over show `sql.instances.restart` and
`sql.instances.failover`. See
[Before a change runs](overview.md#before-a-change-runs). Press `Enter` to
confirm or `Esc` to cancel. On success, the app refreshes the instance list, as
it does for GCE.

The same four actions also run against a marked set. The palette then offers a
command when any marked instance is eligible for it. See
[Batch actions on marked rows](../../guides/batch-actions.md).
